use super::{
    Clip, Damage, Draw, RasterScale, Result, Surface, ViewCaches, draw_bounds, include_changes,
    opaque_cover,
};

const BLOCK_SIZE: usize = 64;

#[derive(Clone, Copy)]
enum Block {
    Changed,
    Unmeasured,
    Measured(Clip),
}

#[derive(Default)]
pub(crate) struct RepairIndex {
    // The final block is always scanned first when looking for an opaque cover.
    blocks: Vec<Block>,
}

impl RepairIndex {
    pub(crate) fn damage(
        &mut self,
        old: &[Draw],
        new: &[Draw],
        scale: RasterScale,
        canvas: Clip,
    ) -> Result<Damage> {
        let mut damage = Damage::default();
        if old.len() != new.len() {
            self.blocks = Vec::new();
            include_changes(old, new, scale, canvas, &mut damage)?;
        } else if self.blocks.is_empty() {
            include_changes(old, new, scale, canvas, &mut damage)?;
            if new.len() >= 2 * BLOCK_SIZE && !damage.is_empty() {
                self.blocks
                    .resize(new.len().div_ceil(BLOCK_SIZE) - 1, Block::Unmeasured);
            }
        } else {
            for (i, (before, after)) in old
                .chunks(BLOCK_SIZE)
                .zip(new.chunks(BLOCK_SIZE))
                .enumerate()
            {
                let changed = include_changes(before, after, scale, canvas, &mut damage)?;
                if let Some(block) = self.blocks.get_mut(i) {
                    if changed {
                        *block = Block::Changed;
                    } else if matches!(block, Block::Changed) {
                        *block = Block::Unmeasured;
                    }
                }
            }
        }
        Ok(damage)
    }

    pub(crate) fn repair(
        &mut self,
        surface: &mut Surface,
        draws: &[Draw],
        scale: RasterScale,
        clip: Clip,
        clear_byte: u8,
        caches: &mut ViewCaches,
    ) -> Result<()> {
        let step = if self.blocks.is_empty() {
            draws.len().max(1)
        } else {
            BLOCK_SIZE
        };
        let mut background = [clear_byte; 4];
        let mut start = 0;
        'search: for (i, block) in draws.chunks(step).enumerate().rev() {
            if let Some(slot) = self.blocks.get_mut(i) {
                if matches!(slot, Block::Unmeasured) {
                    *slot = Block::Measured(draw_bounds(block, scale)?);
                }
                if let Block::Measured(bounds) = slot
                    && bounds.intersect(clip).empty()
                {
                    continue;
                }
            }
            for (offset, draw) in block.iter().enumerate().rev() {
                if let Some(color) = opaque_cover(draw, scale, clip)? {
                    background = [color.red, color.green, color.blue, 255];
                    start = i * step + offset + 1;
                    break 'search;
                }
            }
        }
        surface.clear_region(clip, background);
        let mut replay = caches.replay();
        for (i, block) in draws.chunks(step).enumerate().skip(start / step) {
            if let Some(Block::Measured(bounds)) = self.blocks.get(i)
                && bounds.intersect(clip).empty()
            {
                continue;
            }
            surface.replay_commands(
                &block[start.saturating_sub(i * step)..],
                scale,
                clip,
                &mut replay,
            )?;
        }
        Ok(())
    }
}

impl Surface {
    fn clear_region(&mut self, clip: Clip, background: [u8; 4]) {
        let stride = self.size.width as usize * 4;
        let start = clip.top as usize * stride + clip.left as usize * 4;
        let row = start..start + (clip.right - clip.left) as usize * 4;
        self.rgba[row.clone()]
            .as_chunks_mut::<4>()
            .0
            .fill(background);
        for y in 1..(clip.bottom - clip.top) as usize {
            self.rgba.copy_within(row.clone(), start + y * stride);
        }
    }
}
