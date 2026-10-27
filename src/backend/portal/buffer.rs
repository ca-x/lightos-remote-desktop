//! A dequeued PipeWire buffer with access to the actual metadata allocation
//! length. pipewire-rs' typed cursor helper does not expose that bound.
use super::*;
use std::ptr::NonNull;

pub(super) struct Buffer<'a> {
    raw: NonNull<pw::sys::pw_buffer>,
    stream: &'a pw::stream::Stream,
}
impl<'a> Buffer<'a> {
    pub(super) fn dequeue(stream: &'a pw::stream::Stream) -> Option<Self> {
        // This buffer belongs to the stream until Drop queues it back. No
        // slices escape that lifetime and we never retain pointers to its data.
        let raw = NonNull::new(unsafe { stream.dequeue_raw_buffer() })?;
        Some(Self { raw, stream })
    }
    fn spa(&self) -> Option<&spa::sys::spa_buffer> {
        unsafe { self.raw.as_ref().buffer.as_ref() }
    }
    pub(super) fn cursor(&self) -> Option<&[u8]> {
        let buffer = self.spa()?;
        if buffer.metas.is_null() || buffer.n_metas > 64 {
            return None;
        }
        // PipeWire owns these descriptors and mappings; offsets *inside* a
        // cursor payload are checked separately by the decoder against size.
        let metas = unsafe { std::slice::from_raw_parts(buffer.metas, buffer.n_metas as usize) };
        let meta = metas
            .iter()
            .find(|meta| meta.type_ == spa::sys::SPA_META_Cursor)?;
        if meta.data.is_null() || !(28..=2 * 1024 * 1024).contains(&meta.size) {
            return None;
        }
        Some(unsafe { std::slice::from_raw_parts(meta.data.cast::<u8>(), meta.size as usize) })
    }
    pub(super) fn datas_mut(&mut self) -> &mut [spa::buffer::Data] {
        let Some(buffer) = self.spa() else {
            return &mut [];
        };
        if buffer.datas.is_null() || buffer.n_datas > 64 {
            return &mut [];
        }
        // spa::buffer::Data is repr(transparent) over spa_data. &mut self
        // prevents simultaneous metadata/data borrows while the buffer is held.
        unsafe {
            std::slice::from_raw_parts_mut(
                buffer.datas.cast::<spa::buffer::Data>(),
                buffer.n_datas as usize,
            )
        }
    }
}
impl Drop for Buffer<'_> {
    fn drop(&mut self) {
        unsafe {
            self.stream.queue_raw_buffer(self.raw.as_ptr());
        }
    }
}
