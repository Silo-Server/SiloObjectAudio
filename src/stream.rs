//! What a major sync says about the stream: presentations, channel counts and speakers.

use crate::types::*;
use truehd::process::extract::Frame;
use truehd::process::parse::Parser;
use truehd::process::{MAX_PRESENTATIONS, PresentationMap, PresentationType};
use truehd::structs::channel::ChannelLabel;
use truehd::structs::oamd::SpeakerLabels;
use truehd::structs::sync::MAJOR_SYNC_FBB;

#[derive(Clone, Debug)]
pub struct StreamDesc {
    pub format: u32,
    pub sample_rate: u32,
    pub access_unit_frames: u32,
    pub substreams: u32,
    pub immersive: bool,
    pub has_objects: bool,
    pub map: PresentationMap,
    pub presentations: [PresentationInfo; MAX_PRESENTATIONS],
}

/// Speaker code of a TrueHD channel label. The header's codes are the label order.
pub fn speaker_of_label(label: ChannelLabel) -> u8 {
    use ChannelLabel::*;
    match label {
        L => 0,
        R => 1,
        C => 2,
        LFE => 3,
        Ls => 4,
        Rs => 5,
        Tfl => 6,
        Tfr => 7,
        Tsl => 8,
        Tsr => 9,
        Tbl => 10,
        Tbr => 11,
        Lsc => 12,
        Rsc => 13,
        Lb => 14,
        Rb => 15,
        Cb => 16,
        Tc => 17,
        Lsd => 18,
        Rsd => 19,
        Lw => 20,
        Rw => 21,
        Tfc => 22,
        LFE2 => 23,
    }
}

/// Speaker code of an OAMD bed label index (`BedAssignment` bit order): the 16-channel
/// channel assignment and the OAMD standard bed list group the same speakers the same way.
pub fn speaker_of_oamd_bed(index: usize) -> u8 {
    use SpeakerLabels::*;
    match SpeakerLabels::from_u8(index as u8) {
        Some(L) => 0,
        Some(R) => 1,
        Some(C) => 2,
        Some(LFE) => 3,
        Some(Lss) => 4,
        Some(Rss) => 5,
        Some(Lrs) => 14,
        Some(Rrs) => 15,
        Some(Lfh) => 6,
        Some(Rfh) => 7,
        Some(Lts) => 8,
        Some(Rts) => 9,
        Some(Lrh) => 10,
        Some(Rrh) => 11,
        Some(Lw) => 20,
        Some(Rw) => 21,
        Some(LFE2) => 23,
        None => SPEAKER_UNKNOWN,
    }
}

/// Nominal position of an OAMD bed speaker in room-anchored coordinates. The crate's table is
/// in DAMF coordinates (x -1..1 left to right, y +1 front to -1 back, z as is).
pub fn oamd_bed_position(index: usize) -> [f32; 3] {
    match SpeakerLabels::from_u8(index as u8) {
        Some(label) => {
            let [x, y, z] = *label.pos();
            [(x + 1.0) * 0.5, (1.0 - y) * 0.5, z]
        }
        None => [0.0; 3],
    }
}

pub fn speaker_name(code: u8) -> &'static std::ffi::CStr {
    const NAMES: [&std::ffi::CStr; 24] = [
        c"L", c"R", c"C", c"LFE", c"Ls", c"Rs", c"Tfl", c"Tfr", c"Tsl", c"Tsr", c"Tbl", c"Tbr",
        c"Lsc", c"Rsc", c"Lb", c"Rb", c"Cb", c"Tc", c"Lsd", c"Rsd", c"Lw", c"Rw", c"Tfc",
        c"LFE2",
    ];
    match code {
        0..=23 => NAMES[code as usize],
        254 => c"Obj",
        _ => c"?",
    }
}

/// The parts of a major sync that define the stream's configuration, used to notice when it
/// changes: format sync and format info (rate, channel assignments), flags, substream count
/// and presentation info, and the extra channel meaning (16-channel layout). Left out are
/// fields that legitimately change from one major sync to the next: the DRC start-up gains
/// in `channel_meaning`, the peak data rate and the CRC.
pub fn major_sync_signature(au: &[u8], out: &mut Vec<u8>) -> bool {
    let Some(msi) = (|| {
        Some(if *au.get(7)? == 0xBB {
            26
        } else if au.get(29)? & 1 == 0 {
            26
        } else {
            28 + ((au.get(30)? >> 3) & 0x1E) as usize
        })
    })() else {
        return false;
    };
    let Some(info) = au.get(4..4 + msi) else {
        return false;
    };
    out.clear();
    out.extend_from_slice(&info[0..8]); // format_sync, format_info
    out.extend_from_slice(&info[10..12]); // flags
    out.extend_from_slice(&info[16..18]); // substreams, substream info
    out.extend_from_slice(&info[26..]); // extra channel meaning, if any
    true
}

/// Parses one major sync access unit with every presentation required, so every substream's
/// restart header (and with it every presentation's channel count) is read. Only called when
/// the major sync info changes, so its cost does not recur.
pub fn probe(frame: &Frame) -> Result<StreamDesc, String> {
    let mut parser = Box::new(Parser::default());
    parser.set_check_fifo(false);
    let au = parser.parse(frame).map_err(|e| format!("probe: {e:#}"))?;
    let ms = au
        .major_sync_info
        .as_ref()
        .ok_or_else(|| "probe: not a major sync".to_string())?;

    let is_fbb = ms.format_sync == MAJOR_SYNC_FBB;
    let map =
        PresentationMap::for_format_sync(ms.format_sync, ms.substream_info, ms.extended_substream_info);
    let sample_rate = ms.format_info.sampling_frequency_1().map_err(|e| e.to_string())?;
    let access_unit_frames = ms.format_info.samples_per_au().map_err(|e| e.to_string())? as u32;

    let ecm = ms.channel_meaning.extra_channel_meaning();
    let immersive = !is_fbb && ms.substream_info >> 7 != 0;
    let has_objects = immersive
        && ecm.is_some_and(|e| e.dyn_object_only || e.sixteench_content_description & 0b110 != 0);

    let channels_of = |i: usize| -> u32 {
        au.substream_segment
            .get(i)
            .and_then(|seg| seg.block.first())
            .and_then(|b| b.restart_header.as_ref())
            .map_or(0, |rh| rh.max_matrix_chan as u32 + 1)
    };

    let mut presentations = [PresentationInfo::ABSENT; MAX_PRESENTATIONS];
    for (i, info) in presentations.iter_mut().enumerate() {
        let (ptype, source, decoded_as) = match map.presentation_type_by_index(i) {
            PresentationType::Invalid => continue,
            PresentationType::Independent => (PTYPE_INDEPENDENT, -1, i),
            PresentationType::DownmixOf(j) => (PTYPE_DOWNMIX, j as i32, i),
            PresentationType::CopyOf(j) => (PTYPE_COPY, j as i32, j),
        };
        if decoded_as >= ms.substreams {
            continue;
        }
        info.ptype = ptype;
        info.source = source;
        info.channel_count = channels_of(decoded_as).min(MAX_CHANNELS as u32);
        let labels = au.get_channel_labels(decoded_as).unwrap_or_default();
        let objects = decoded_as == 3 && has_objects;
        info.speakers = roles(&labels, info.channel_count as usize, objects);
    }

    Ok(StreamDesc {
        format: if is_fbb { FORMAT_MLP } else { FORMAT_TRUEHD },
        sample_rate,
        access_unit_frames,
        substreams: ms.substreams as u32,
        immersive,
        has_objects,
        map,
        presentations,
    })
}

/// Channel roles of a presentation: its labelled bed channels first, then, for an object
/// presentation, objects (ISF, then dynamic); UNKNOWN where the stream says nothing.
pub fn roles(labels: &[ChannelLabel], channels: usize, objects: bool) -> [u8; MAX_CHANNELS] {
    let mut speakers = [SPEAKER_UNKNOWN; MAX_CHANNELS];
    for (i, slot) in speakers.iter_mut().enumerate().take(channels) {
        *slot = match labels.get(i) {
            Some(&label) => speaker_of_label(label),
            None if objects => SPEAKER_OBJECT,
            None => SPEAKER_UNKNOWN,
        };
    }
    speakers
}

/// The presentation a selector decodes on this stream.
pub fn resolve(selection: i32, map: &PresentationMap) -> usize {
    let pick = |i: usize| match map.presentation_type_by_index(i) {
        PresentationType::Invalid => None,
        PresentationType::CopyOf(j) => Some(j),
        _ => Some(i),
    };
    let highest = || map.max_independent_presentation().unwrap_or(0);
    match selection {
        PRESENTATION_HIGHEST => pick(3).unwrap_or_else(highest),
        PRESENTATION_HIGHEST_CHANNEL_BASED => (0..=2).rev().find_map(pick).unwrap_or(0),
        n => pick(n as usize).unwrap_or_else(highest),
    }
}

pub fn valid_selection(selection: i32) -> bool {
    matches!(selection, -2..=3)
}
