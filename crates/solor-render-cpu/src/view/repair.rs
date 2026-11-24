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
    backdrop_inputs: std::collections::BTreeMap<usize, Clip>,
}

impl RepairIndex {
    pub(crate) fn new(draws: &[Draw], scale: RasterScale, canvas: Clip) -> Result<Self> {
        let mut index = Self::default();
        for (i, draw) in draws.iter().enumerate() {
            if let Some(input) = super::backdrop::footprint(draw, scale, canvas)? {
                index.backdrop_inputs.insert(i, input);
            }
        }
        Ok(index)
    }

    pub(crate) fn damage(
        &mut self,
        previous: &solor::ViewFrame,
        current: &solor::ViewFrame,
        scale: RasterScale,
        canvas: Clip,
    ) -> Result<Damage> {
        let (old, new) = (previous.draws(), current.draws());
        let (prefix, suffix) = current.common_draw_edges(previous);
        let before = &old[prefix..old.len() - suffix];
        let after = &new[prefix..new.len() - suffix];
        let mut damage = Damage::default();
        let inputs = &mut self.backdrop_inputs;
        let mut compare = |offset, before: &[Draw], after: &[Draw], damage: &mut Damage| {
            include_changes(before, after, scale, canvas, damage, &mut |i, draw| {
                if let Some(input) = super::backdrop::footprint(draw, scale, canvas)? {
                    inputs.insert(offset + i, input);
                } else if !inputs.is_empty() {
                    inputs.remove(&(offset + i));
                }
                Ok(())
            })
        };
        if old.len() != new.len() {
            *self = Self::new(new, scale, canvas)?;
            include_changes(
                before,
                after,
                scale,
                canvas,
                &mut damage,
                &mut |_, _| Ok(()),
            )?;
        } else if self.blocks.is_empty() {
            compare(prefix, before, after, &mut damage)?;
            if new.len() >= 2 * BLOCK_SIZE && !damage.is_empty() {
                self.blocks
                    .resize(new.len().div_ceil(BLOCK_SIZE) - 1, Block::Unmeasured);
            }
        } else if !after.is_empty() {
            for (i, (before, after)) in old
                .chunks(BLOCK_SIZE)
                .zip(new.chunks(BLOCK_SIZE))
                .enumerate()
                .take((new.len() - suffix).div_ceil(BLOCK_SIZE))
                .skip(prefix / BLOCK_SIZE)
            {
                let changed = compare(i * BLOCK_SIZE, before, after, &mut damage)?;
                if let Some(block) = self.blocks.get_mut(i) {
                    if changed {
                        *block = Block::Changed;
                    } else if matches!(block, Block::Changed) {
                        *block = Block::Unmeasured;
                    }
                }
            }
        }
        // A repair that intersects a filter must reconstruct its source pixels.
        // Merging nearby repairs may reach another filter, so repeat until stable.
        while self.backdrop_inputs.values().any(|input| {
            damage
                .rects()
                .iter()
                .any(|rect| !rect.intersect(*input).empty() && rect.union(*input) != *rect)
        }) {
            let mut expanded = Damage::default();
            for &rect in damage.rects() {
                let area = self.backdrop_inputs.values().fold(rect, |area, input| {
                    if area.intersect(*input).empty() {
                        area
                    } else {
                        area.union(*input)
                    }
                });
                expanded.include(area);
            }
            damage = expanded;
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
