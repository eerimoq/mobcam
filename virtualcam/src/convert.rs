//! Turns a decoded frame into the I420 image a v4l2loopback device is fed.

use mobcam_core::ffmpeg::{self, sys as av};

/// How to read one output plane out of a decoder plane: `columns` samples from
/// each of `rows` rows, taking every `column_step`-th sample of every
/// `row_step`-th row, starting `offset` samples into each row.
struct Grid {
    columns: usize,
    rows: usize,
    column_step: usize,
    row_step: usize,
    offset: usize,
}

impl Grid {
    /// The plane holds exactly one sample per output sample, in order.
    fn direct(columns: usize, rows: usize) -> Self {
        Self {
            columns,
            rows,
            column_step: 1,
            row_step: 1,
            offset: 0,
        }
    }

    /// One of the two components of an interleaved chroma plane.
    fn component(&self, offset: usize) -> Self {
        Self {
            columns: self.columns,
            rows: self.rows,
            column_step: 2,
            row_step: self.row_step,
            offset,
        }
    }

    /// The rows of the decoder plane the grid reaches into.
    fn source_rows(&self) -> Option<usize> {
        self.rows.checked_sub(1)?.checked_mul(self.row_step)?.checked_add(1)
    }

    /// The samples of a row the grid reaches into, the offset included.
    fn source_columns(&self) -> Option<usize> {
        self.columns
            .checked_sub(1)?
            .checked_mul(self.column_step)?
            .checked_add(self.offset + 1)
    }
}

struct Plane<'a> {
    data: &'a [u8],
    stride: usize,
}

/// The frame size I420 allows, which is the decoded one rounded down to even.
fn size(frame: &ffmpeg::Frame) -> Option<(usize, usize)> {
    let width = usize::try_from(frame.width()).ok()? & !1;
    let height = usize::try_from(frame.height()).ok()? & !1;
    (width != 0 && height != 0).then_some((width, height))
}

/// Writes the frame into `out` as I420 and returns the size it was written at,
/// or nothing if the pixel format is not one this understands.
pub fn to_i420(frame: &ffmpeg::Frame, out: &mut Vec<u8>) -> Option<(u32, u32)> {
    let (width, height) = size(frame)?;
    let (chroma_width, chroma_height) = (width / 2, height / 2);
    out.clear();
    out.reserve(width * height + 2 * chroma_width * chroma_height);
    let luma = Grid::direct(width, height);
    let chroma = Grid::direct(chroma_width, chroma_height);
    let converted = match frame.pixel_format() {
        av::AV_PIX_FMT_YUV420P | av::AV_PIX_FMT_YUVJ420P => {
            planar(out, frame, [(0, &luma, 0), (1, &chroma, 0), (2, &chroma, 0)])
        }
        av::AV_PIX_FMT_NV12 => planar(out, frame, [(0, &luma, 0)]) && interleaved(out, frame, &chroma, 0, [0, 1]),
        av::AV_PIX_FMT_NV21 => planar(out, frame, [(0, &luma, 0)]) && interleaved(out, frame, &chroma, 0, [1, 0]),
        // 4:2:2 keeps every other chroma row, and 4:4:4 also keeps every other
        // chroma column.
        av::AV_PIX_FMT_YUV422P => {
            let chroma = Grid { row_step: 2, ..chroma };
            planar(out, frame, [(0, &luma, 0), (1, &chroma, 0), (2, &chroma, 0)])
        }
        av::AV_PIX_FMT_YUV444P => {
            let chroma = Grid {
                column_step: 2,
                row_step: 2,
                ..chroma
            };
            planar(out, frame, [(0, &luma, 0), (1, &chroma, 0), (2, &chroma, 0)])
        }
        // Ten bit samples sit in the low bits for I010 and in the high bits for
        // P010, so they need a different shift to become eight bit ones.
        av::AV_PIX_FMT_YUV420P10LE => planar(out, frame, [(0, &luma, 2), (1, &chroma, 2), (2, &chroma, 2)]),
        av::AV_PIX_FMT_P010LE => planar(out, frame, [(0, &luma, 8)]) && interleaved(out, frame, &chroma, 8, [0, 1]),
        _ => false,
    };
    converted.then_some((width as u32, height as u32))
}

/// Copies whole planes, one output plane each. A `shift` of zero means the
/// plane holds eight bit samples, anything else sixteen bit little endian ones.
fn planar<const N: usize>(out: &mut Vec<u8>, frame: &ffmpeg::Frame, planes: [(usize, &Grid, u32); N]) -> bool {
    planes
        .into_iter()
        .all(|(index, grid, shift)| plane(frame, index, grid).is_some_and(|plane| copy(out, &plane, grid, shift)))
}

/// Splits an interleaved chroma plane into the two output planes, taking the
/// component at the first offset for all of the first plane and so on.
fn interleaved(out: &mut Vec<u8>, frame: &ffmpeg::Frame, grid: &Grid, shift: u32, offsets: [usize; 2]) -> bool {
    let Some(plane) = plane(frame, 1, grid) else {
        return false;
    };
    offsets
        .into_iter()
        .all(|offset| copy(out, &plane, &grid.component(offset), shift))
}

fn plane<'a>(frame: &'a ffmpeg::Frame, index: usize, grid: &Grid) -> Option<Plane<'a>> {
    let (data, linesize) = frame.plane(index);
    let stride = usize::try_from(linesize).ok()?;
    if data.is_null() || stride == 0 {
        return None;
    }
    let length = stride.checked_mul(grid.source_rows()?)?;
    Some(Plane {
        data: unsafe { std::slice::from_raw_parts(data, length) },
        stride,
    })
}

fn copy(out: &mut Vec<u8>, plane: &Plane<'_>, grid: &Grid, shift: u32) -> bool {
    let sample_size = if shift == 0 { 1 } else { 2 };
    let Some(columns) = grid.source_columns() else {
        return false;
    };
    for row in 0..grid.rows {
        let start = row * grid.row_step * plane.stride + grid.offset * sample_size;
        let length = (columns - grid.offset) * sample_size;
        let Some(source) = plane.data.get(start..).and_then(|rest| rest.get(..length)) else {
            return false;
        };
        if shift == 0 {
            out.extend(source.iter().step_by(grid.column_step));
        } else {
            out.extend(
                source
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .step_by(grid.column_step)
                    .map(|sample| (u16::from_le_bytes(*sample) >> shift) as u8),
            );
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copied(plane: &[u8], stride: usize, grid: &Grid, shift: u32) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let plane = Plane { data: plane, stride };
        copy(&mut out, &plane, grid, shift).then_some(out)
    }

    #[test]
    fn whole_rows_ignore_the_padding_a_stride_adds() {
        let plane = [1, 2, 3, 99, 4, 5, 6, 99];
        let grid = Grid::direct(3, 2);
        assert_eq!(copied(&plane, 4, &grid, 0), Some(vec![1, 2, 3, 4, 5, 6]));
    }

    #[test]
    fn interleaved_chroma_splits_into_two_planes() {
        // One row of U0 V0 U1 V1, with a byte of stride padding.
        let plane = [10, 20, 11, 21, 99];
        let grid = Grid::direct(2, 1);
        assert_eq!(copied(&plane, 5, &grid.component(0), 0), Some(vec![10, 11]));
        assert_eq!(copied(&plane, 5, &grid.component(1), 0), Some(vec![20, 21]));
    }

    #[test]
    fn every_other_row_and_column_is_kept() {
        let plane: Vec<u8> = (0..16).collect();
        let grid = Grid {
            columns: 2,
            rows: 2,
            column_step: 2,
            row_step: 2,
            offset: 0,
        };
        assert_eq!(copied(&plane, 4, &grid, 0), Some(vec![0, 2, 8, 10]));
    }

    #[test]
    fn ten_bit_samples_narrow_to_eight() {
        // 1023 and 512 in the low bits, as I010 stores them.
        let plane = [0xff, 0x03, 0x00, 0x02];
        assert_eq!(copied(&plane, 4, &Grid::direct(2, 1), 2), Some(vec![255, 128]));
        // The same values in the high bits, as P010 stores them.
        let plane = [0xc0, 0xff, 0x00, 0x80];
        assert_eq!(copied(&plane, 4, &Grid::direct(2, 1), 8), Some(vec![255, 128]));
    }

    #[test]
    fn a_plane_that_is_too_short_is_refused() {
        assert_eq!(copied(&[1, 2, 3], 4, &Grid::direct(4, 1), 0), None);
        assert_eq!(copied(&[1, 2, 3, 4], 4, &Grid::direct(3, 2), 0), None);
    }

    #[test]
    fn the_needed_source_size_counts_the_steps() {
        let grid = Grid {
            columns: 3,
            rows: 4,
            column_step: 2,
            row_step: 2,
            offset: 1,
        };
        assert_eq!(grid.source_rows(), Some(7));
        assert_eq!(grid.source_columns(), Some(6));
    }
}
