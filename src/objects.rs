//! Object audio metadata: turning OAMD payloads into timed element updates and handing each
//! update out with the block that contains its start.

use crate::stream::{oamd_bed_position, speaker_of_oamd_bed};
use crate::types::*;
use std::collections::VecDeque;
use truehd::structs::oamd::{GAIN_MINUS_INFINITY, ObjectAudioMetadataPayload};

/// Updates waiting for their block. Payloads arrive every ~38 access units and state at most
/// eight updates each, none due more than a few thousand samples ahead; anything beyond this
/// is a malformed stream and is dropped rather than buffered without bound.
const MAX_PENDING: usize = 64;
const MAX_UPDATES_PER_BLOCK: usize = 16;

pub type Elements = [Element; MAX_CHANNELS];

/// Channel roles as the OAMD states them: element kinds, speakers and counts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Layout {
    pub element_count: usize,
    pub bed_count: usize,
    pub isf_count: usize,
    pub dynamic_count: usize,
    pub speakers: [u8; MAX_CHANNELS],
}

struct Pending {
    due: u64,
    ramp: u32,
    layout: Layout,
    elements: Elements,
}

pub struct ObjectState {
    have_state: bool,
    layout: Option<Layout>,
    warp_mode: i32,
    current: Box<Elements>,
    pending: VecDeque<Pending>,
    block_elements: Vec<Elements>,
    block_updates: Vec<MetadataUpdate>,
    metadata: Metadata,
    bed_labels: Vec<usize>,
}

pub struct BlockMetadata {
    pub metadata: *const Metadata,
    pub update_count: usize,
}

impl Default for ObjectState {
    fn default() -> Self {
        Self {
            have_state: false,
            layout: None,
            warp_mode: WARP_NOT_SIGNALLED,
            current: Box::new([Element::EMPTY; MAX_CHANNELS]),
            pending: VecDeque::with_capacity(MAX_PENDING),
            block_elements: Vec::with_capacity(MAX_UPDATES_PER_BLOCK),
            block_updates: Vec::with_capacity(MAX_UPDATES_PER_BLOCK),
            metadata: Metadata {
                element_count: 0,
                bed_count: 0,
                isf_count: 0,
                dynamic_count: 0,
                warp_mode: WARP_NOT_SIGNALLED,
                update_count: 0,
                updates: std::ptr::null(),
                elements: std::ptr::null(),
            },
            bed_labels: Vec::with_capacity(MAX_CHANNELS + 1),
        }
    }
}

impl ObjectState {
    /// The element configuration of the updates delivered so far, if any since reset.
    pub fn layout(&self) -> Option<Layout> {
        self.layout.filter(|_| self.have_state)
    }

    pub fn clear(&mut self) {
        self.have_state = false;
        self.layout = None;
        self.warp_mode = WARP_NOT_SIGNALLED;
        self.pending.clear();
        self.block_elements.clear();
        self.block_updates.clear();
    }

    /// Schedules the updates of one payload that arrived with the access unit starting at
    /// stream frame `au_start`, carried at `evo_offset` frames into it.
    pub fn ingest(
        &mut self,
        payload: &ObjectAudioMetadataPayload,
        au_start: u64,
        evo_offset: u64,
        channel_count: usize,
    ) -> Result<(), String> {
        if let Some(trim) = &payload.trim_element {
            self.warp_mode = trim.warp_mode as i32;
        }
        let Some(object_element) = &payload.object_element else {
            return Ok(()); // a payload without object data only restates trims or the like
        };

        let prog = &payload.program_assignment;
        let count = payload.object_count;
        if count != channel_count || count > MAX_CHANNELS {
            return Err(format!(
                "OAMD describes {count} elements but the presentation decodes {channel_count} channels"
            ));
        }
        self.bed_labels.clear();
        for bed in &prog.bed_assignment {
            self.bed_labels.extend(bed.to_index_vec());
        }
        let bed_count = prog.num_bed_objects;
        let isf_count = prog.num_isf_objects;
        if self.bed_labels.len() != bed_count || bed_count + isf_count > count {
            return Err(format!(
                "OAMD bed assignment inconsistent: {} labels, {bed_count} bed objects, {isf_count} ISF, {count} elements",
                self.bed_labels.len()
            ));
        }

        let mut layout = Layout {
            element_count: count,
            bed_count,
            isf_count,
            dynamic_count: count - bed_count - isf_count,
            speakers: [SPEAKER_UNKNOWN; MAX_CHANNELS],
        };
        for (i, speaker) in layout.speakers.iter_mut().enumerate().take(count) {
            *speaker = if i < bed_count {
                speaker_of_oamd_bed(self.bed_labels[i])
            } else {
                SPEAKER_OBJECT
            };
        }

        let info = &object_element.md_update_info;
        let blocks = info.num_obj_info_blocks;
        if info.block_update_info.len() != blocks
            || object_element.object_data.len() != count
            || object_element.object_data.iter().any(|o| o.len() != blocks)
        {
            return Err("OAMD object data does not match its update count".into());
        }

        for (blk, update) in info.block_update_info.iter().enumerate() {
            // ETSI TS 103 420 clause 5.3: t = sample_offset + 32 * block_offset_factor.
            let due = au_start
                + evo_offset
                + info.sample_offset as u64
                + 32 * update.block_offset_factor_bits as u64;
            let mut elements = [Element::EMPTY; MAX_CHANNELS];
            for (i, element) in elements.iter_mut().enumerate().take(count) {
                *element = self.element(payload, &layout, i, blk);
            }
            if self.pending.len() >= MAX_PENDING {
                return Err("too many pending OAMD updates".into());
            }
            let at = self.pending.partition_point(|p| p.due <= due);
            self.pending.insert(
                at,
                Pending {
                    due,
                    ramp: update.ramp_duration as u32,
                    layout,
                    elements,
                },
            );
        }
        Ok(())
    }

    fn element(
        &self,
        p: &ObjectAudioMetadataPayload,
        layout: &Layout,
        i: usize,
        blk: usize,
    ) -> Element {
        let od = &p.object_element.as_ref().expect("checked by caller").object_data[i][blk];
        let mut e = Element::EMPTY;

        if i < layout.bed_count {
            let label = self.bed_labels[i];
            e.speaker = layout.speakers[i];
            e.kind = if e.speaker == SPEAKER_LFE || e.speaker == SPEAKER_LFE2 {
                ELEMENT_LFE
            } else {
                ELEMENT_BED
            };
            e.position = oamd_bed_position(label);
        } else if i < layout.bed_count + layout.isf_count {
            e.kind = ELEMENT_ISF;
        } else {
            e.kind = ELEMENT_OBJECT;
        }

        if !od.b_object_not_active {
            e.flags |= EL_ACTIVE;
        }
        let basic = &od.object_basic_info;
        if basic.object_gain == GAIN_MINUS_INFINITY {
            e.gain = 0.0;
            e.gain_db = f32::NEG_INFINITY;
        } else {
            e.gain_db = basic.object_gain as f32;
            e.gain = 10f32.powf(e.gain_db / 20.0);
        }
        e.priority = basic.object_priority as f32;

        if e.kind == ELEMENT_OBJECT {
            let r = &od.object_render_info;
            let mut pos = r.pos3d;
            if let Some(ext) = p
                .extended_object_element
                .as_ref()
                .and_then(|x| x.ext_prec_pos_block.get(i))
                .and_then(|o| o.get(blk))
            {
                pos[0] += ext.ext_prec_pos3d_x;
                pos[1] += ext.ext_prec_pos3d_y;
                pos[2] += ext.ext_prec_pos3d_z;
            }
            e.position = [
                pos[0].clamp(0.0, 1.0) as f32,
                pos[1].clamp(0.0, 1.0) as f32,
                pos[2].clamp(-1.0, 1.0) as f32,
            ];
            e.zone = r.zone_constraints_idx;
            if r.b_enable_elevation {
                e.flags |= EL_ELEVATION;
            }
            if r.b_object_snap {
                e.flags |= EL_SNAP;
            }
            e.size = r.object_size.map(|s| s as f32);
            if r.b_object_use_screen_ref {
                e.flags |= EL_SCREEN_REF;
                e.screen_factor = r.screen_factor as f32;
            }
            e.depth_factor = r.depth_factor as f32;
            if r.b_object_distance_specified {
                e.flags |= EL_DISTANCE;
                e.distance = r.distance_factor.unwrap_or(0.0) as f32;
            }
            if let Some(div) = p
                .extended_object_element
                .as_ref()
                .filter(|x| x.b_obj_div_block)
                .and_then(|x| x.object_div_block.get(i))
                .and_then(|o| o.get(blk))
                .filter(|d| d.b_object_divergence)
            {
                e.flags |= EL_DIVERGENCE;
                e.divergence = div.object_divergence as f32;
            }
        }

        // Trim bypass as truehdd's DAMF writer derives it.
        if let Some(trim) = &p.trim_element {
            let bypass = if trim.b_disable_trim_per_obj {
                trim.b_disable_trim.get(i).copied().unwrap_or(false)
            } else {
                trim.global_trim_mode == 1
            };
            if bypass {
                e.flags |= EL_TRIM_BYPASS;
            }
        }
        if let Some(hp) = p
            .headphone_element
            .as_ref()
            .and_then(|h| h.headphone_data.get(i))
            .and_then(|o| o.get(blk.min(o.len().saturating_sub(1))))
        {
            e.headphone_mode = hp.hp_render_mode;
            if hp.hp_head_track_disable {
                e.flags |= EL_HEAD_TRACK_DISABLE;
            }
        }
        if let Some(&dialog) = p
            .object_description_element
            .as_ref()
            .and_then(|d| d.object_dialog_indication.get(i))
            .and_then(|o| o.get(blk.min(o.len().saturating_sub(1))))
        {
            e.dialog = dialog;
        }
        e
    }

    /// Moves the updates starting within `[start, start + frames)` into this block and returns
    /// the metadata to attach, or a null pointer before the first update since reset.
    pub fn begin_block(&mut self, start: u64, frames: u32) -> BlockMetadata {
        self.block_elements.clear();
        self.block_updates.clear();
        let end = start + frames as u64;
        let mut offsets: [(u32, u32); MAX_UPDATES_PER_BLOCK] = [(0, 0); MAX_UPDATES_PER_BLOCK];

        while self.pending.front().is_some_and(|p| p.due < end) {
            let p = self.pending.pop_front().expect("front checked");
            if self.block_elements.len() == MAX_UPDATES_PER_BLOCK {
                // Pathological: fold into the newest state without its own update entry.
                *self.current = p.elements;
                continue;
            }
            let count = p.layout.element_count;
            let mut elements = p.elements;
            let layout_changed = self.layout != Some(p.layout);
            for (e, cur) in elements.iter_mut().zip(self.current.iter()).take(count) {
                e.flags &= !EL_CHANGED;
                if !self.have_state || layout_changed || !e.same_values(cur) {
                    e.flags |= EL_CHANGED;
                }
            }
            if layout_changed {
                self.layout = Some(p.layout);
            }
            *self.current = elements;
            for e in self.current.iter_mut() {
                e.flags &= !EL_CHANGED;
            }
            self.have_state = true;
            offsets[self.block_elements.len()] = (p.due.saturating_sub(start) as u32, p.ramp);
            self.block_elements.push(elements);
        }

        // Pointers are taken only now that block_elements will not grow again.
        for (i, elements) in self.block_elements.iter().enumerate() {
            self.block_updates.push(MetadataUpdate {
                frame_offset: offsets[i].0,
                ramp_frames: offsets[i].1,
                elements: elements.as_ptr(),
            });
        }

        let Some(layout) = self.layout.filter(|_| self.have_state) else {
            return BlockMetadata {
                metadata: std::ptr::null(),
                update_count: 0,
            };
        };
        self.metadata = Metadata {
            element_count: layout.element_count as u32,
            bed_count: layout.bed_count as u32,
            isf_count: layout.isf_count as u32,
            dynamic_count: layout.dynamic_count as u32,
            warp_mode: self.warp_mode,
            update_count: self.block_updates.len() as u32,
            updates: if self.block_updates.is_empty() {
                std::ptr::null()
            } else {
                self.block_updates.as_ptr()
            },
            elements: self.current.as_ptr(),
        };
        BlockMetadata {
            metadata: &self.metadata,
            update_count: self.block_updates.len(),
        }
    }
}
