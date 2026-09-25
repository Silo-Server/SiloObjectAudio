//! Compare update-time interpretations: with and without 32 * block_offset_factor.
use std::collections::BTreeMap;
use truehd::process::decode::Decoder;
use truehd::process::extract::Extractor;
use truehd::process::parse::Parser;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("path");
    let data = std::fs::read(&path)?;
    let mut ex = Extractor::default();
    ex.push_bytes(&data);
    let mut parser = Parser::default();
    parser.set_check_fifo(false);
    parser.set_required_presentations(&[false, false, false, true]);
    let mut dec = Decoder::default();
    let mut pos = 0u64;
    let (mut with_bo, mut without_bo) = (Vec::new(), Vec::new());
    for f in ex.by_ref() {
        let Ok(f) = f else { break };
        let au = parser.parse(&f)?;
        let d = dec.decode_presentations(&au, &[false, false, false, true])?;
        let d = d[3].as_ref().unwrap();
        for p in &d.oamd {
            if let Some(oe) = &p.object_element {
                let so = oe.md_update_info.sample_offset as u64 + p.evo_sample_offset;
                let bo = oe.md_update_info.block_update_info[0].block_offset_factor_bits as u64;
                with_bo.push(pos + so + 32 * bo);
                without_bo.push(pos + so);
            }
        }
        pos += d.sample_length as u64;
    }
    for (name, v) in [("so+32*bo", &with_bo), ("so only", &without_bo)] {
        let mut h: BTreeMap<i64, u64> = BTreeMap::new();
        for w in v.windows(2) {
            *h.entry(w[1] as i64 - w[0] as i64).or_default() += 1;
        }
        println!("{name}: first={:?} intervals={h:?}", v.first());
    }
    Ok(())
}
