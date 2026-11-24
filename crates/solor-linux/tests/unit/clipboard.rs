use super::*;
use std::rc::Rc;

#[test]
fn counts_utf8_bytes_across_pipe_reads() -> Result<()> {
    let focus = Rc::new(());
    for limit in [Some(6), Some(5), None] {
        let mut transfers = Transfers::default();
        let (fd, mut result) = transfers.read(Rc::downgrade(&focus), limit)?;
        let mut writer = File::from(fd);
        for part in ["你", "好"] {
            writer.write_all(part.as_bytes())?;
            transfers.progress(&[0]);
        }
        if limit == Some(5) {
            assert!(result.try_recv()?.unwrap().is_err());
        } else {
            drop(writer);
            transfers.progress(&[0]);
            assert_eq!(result.try_recv()?.unwrap()?, "你好");
        }
    }
    Ok(())
}
