//! Upload changed aligned blocks while retaining the existing GPU allocation.
//! Callers pass an empty previous slice after replacing the GPU buffer.
pub(super) fn upload_changed(
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    previous: &[u8],
    next: &[u8],
) -> usize {
    debug_assert_eq!(next.len() % wgpu::COPY_BUFFER_ALIGNMENT as usize, 0);
    let mut uploaded = 0;
    let mut start = None;
    // Coarse blocks avoid a separate queue write for each changed float.
    for offset in (0..next.len()).step_by(64) {
        let end = (offset + 64).min(next.len());
        if previous.get(offset..end) != Some(&next[offset..end]) {
            start.get_or_insert(offset);
        } else if let Some(first) = start.take() {
            queue.write_buffer(buffer, first as u64, &next[first..offset]);
            uploaded += offset - first;
        }
    }
    if let Some(first) = start {
        queue.write_buffer(buffer, first as u64, &next[first..]);
        uploaded += next.len() - first;
    }
    uploaded
}
