//! Survey of how a stream carries object metadata: payload cadence, timing fields, and
//! whether the element count matches the decoded channel count. Used to size the C API.
use std::collections::BTreeMap;
use truehd::process::decode::Decoder;
use truehd::process::extract::Extractor;
use truehd::process::parse::Parser;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("path to .thd");
    let data = std::fs::read(&path)?;
    let mut extractor = Extractor::default();
    let mut parser = Parser::default();
    parser.set_check_fifo(false);
    parser.set_required_presentations(&[false, false, false, true]);
    let mut decoder = Decoder::default();

    let mut au = 0u64;
    let mut samples = 0u64;
    let mut last_payload_au: Option<u64> = None;
    let mut gaps: BTreeMap<u64, u64> = BTreeMap::new();
    let mut per_au: BTreeMap<usize, u64> = BTreeMap::new();
    let mut so: BTreeMap<usize, u64> = BTreeMap::new();
    let mut evo: BTreeMap<u64, u64> = BTreeMap::new();
    let mut blocks: BTreeMap<usize, u64> = BTreeMap::new();
    let mut ramps: BTreeMap<u16, u64> = BTreeMap::new();
    let mut bo: BTreeMap<u8, u64> = BTreeMap::new();
    let mut majors = 0u64;
    let mut mismatch = 0u64;
    let mut dup = 0u64;

    for chunk in data.chunks(4096) {
        extractor.push_bytes(chunk);
        loop {
            let frame = match extractor.next() {
                Some(Ok(f)) => f,
                Some(Err(truehd::utils::errors::ExtractError::InsufficientData)) | None => break,
                Some(Err(e)) => {
                    eprintln!("extract: {e}");
                    continue;
                }
            };
            if frame.is_major_sync() {
                majors += 1;
            }
            let access_unit = parser.parse(&frame)?;
            let decoded = decoder.decode_presentations(&access_unit, &[false, false, false, true])?;
            let d = decoded[3].as_ref().expect("presentation 3");
            if d.is_duplicate {
                dup += 1;
            }
            *per_au.entry(d.oamd.len()).or_default() += 1;
            if !d.oamd.is_empty() {
                if let Some(p) = last_payload_au {
                    *gaps.entry(au - p).or_default() += 1;
                }
                last_payload_au = Some(au);
            }
            for p in &d.oamd {
                *evo.entry(p.evo_sample_offset).or_default() += 1;
                if p.object_count != d.channel_count {
                    mismatch += 1;
                }
                if let Some(oe) = &p.object_element {
                    *so.entry(oe.md_update_info.sample_offset).or_default() += 1;
                    *blocks.entry(oe.md_update_info.num_obj_info_blocks).or_default() += 1;
                    for b in &oe.md_update_info.block_update_info {
                        *ramps.entry(b.ramp_duration).or_default() += 1;
                        *bo.entry(b.block_offset_factor_bits).or_default() += 1;
                    }
                }
            }
            samples += d.sample_length as u64;
            au += 1;
        }
    }
    println!("{path}: {au} AUs, {samples} samples, {majors} major syncs, {dup} duplicates");
    println!("payloads per AU: {per_au:?}");
    println!("AU gap between payload-carrying AUs: {gaps:?}");
    println!("evo smploffst: {evo:?}");
    println!("oamd sample_offset: {so:?}");
    println!("num_obj_info_blocks: {blocks:?}");
    println!("block_offset_factor: {bo:?}");
    println!("ramp_duration: {ramps:?}");
    println!("object_count != channel_count: {mismatch}");
    Ok(())
}
