use crate::v4l2::Format;
use mobcam_core::ffmpeg::{self, sys as av};

struct Grid {
    columns: usize,
    rows: usize,
    column_step: usize,
    row_step: usize,
    offset: usize,
}

impl Grid {
    fn direct(columns: usize, rows: usize) -> Self {
        Self {
            columns,
            rows,
            column_step: 1,
            row_step: 1,
            offset: 0,
        }
    }

    fn component(&self, offset: usize) -> Self {
        Self {
            columns: self.columns,
            rows: self.rows,
            column_step: 2,
            row_step: self.row_step,
            offset,
        }
    }

    fn source_rows(&self) -> Option<usize> {
        self.rows.checked_sub(1)?.checked_mul(self.row_step)?.checked_add(1)
    }

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

/// The buffer an image is being laid out in, a plane at a time.
struct Out<'a> {
    data: &'a mut [u8],
    filled: usize,
}

impl<'a> Out<'a> {
    fn new(data: &'a mut [u8]) -> Self {
        Self { data, filled: 0 }
    }

    /// Take the next `length` bytes to lay a row in, if there are that many.
    fn take(&mut self, length: usize) -> Option<&mut [u8]> {
        let end = self.filled.checked_add(length)?;
        let room = self.data.get_mut(self.filled..end)?;
        self.filled = end;
        Some(room)
    }
}

fn dimensions(frame: &ffmpeg::Frame) -> Option<(usize, usize)> {
    let width = usize::try_from(frame.width()).ok()? & !1;
    let height = usize::try_from(frame.height()).ok()? & !1;
    (width != 0 && height != 0).then_some((width, height))
}

/// How the decoder laid a frame out, as the one thing that decides what has to
/// be copied where. Naming the layout rather than the pixel format keeps the
/// choice of it and the carrying of it out from drifting apart.
#[derive(Clone, Copy)]
enum Source {
    /// A plane per component, the chroma at half the width and height.
    Planar,
    /// The same, with the chroma at full height.
    PlanarTall,
    /// The same, at full width and height.
    PlanarWide,
    /// The same, ten bits a sample in the low bits.
    PlanarTen,
    /// A luma plane and a plane of chroma pairs.
    SemiPlanar,
    /// The same, the pairs the other way round.
    SemiPlanarSwapped,
    /// The same, ten bits a sample in the high bits.
    SemiPlanarTen,
    /// A luma plane and a plane of chroma pairs, kept as they are.
    AsIs,
}

/// What a decoded frame becomes when it is written to a camera.
#[derive(Clone, Copy)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub format: Format,
    source: Source,
}

/// Work out what a decoded frame becomes, without copying anything yet.
///
/// `nv12` says whether the device takes interleaved chroma. When it does, a
/// frame that already is NV12 keeps its chroma as it is, which is the whole of
/// what a hardware decoder hands over and saves splitting it into two planes
/// only for the program reading the camera to weave it back together. Every
/// other frame becomes I420, which every device takes.
pub fn image(frame: &ffmpeg::Frame, nv12: bool) -> Option<Image> {
    let (width, height) = dimensions(frame)?;
    let (format, source) = match frame.pixel_format() {
        av::AV_PIX_FMT_NV12 if nv12 => (Format::Nv12, Source::AsIs),
        av::AV_PIX_FMT_YUV420P | av::AV_PIX_FMT_YUVJ420P => (Format::I420, Source::Planar),
        av::AV_PIX_FMT_NV12 => (Format::I420, Source::SemiPlanar),
        av::AV_PIX_FMT_NV21 => (Format::I420, Source::SemiPlanarSwapped),
        // 4:2:2 keeps every other chroma row, and 4:4:4 also keeps every other
        // chroma column.
        av::AV_PIX_FMT_YUV422P => (Format::I420, Source::PlanarTall),
        av::AV_PIX_FMT_YUV444P => (Format::I420, Source::PlanarWide),
        av::AV_PIX_FMT_YUV420P10LE => (Format::I420, Source::PlanarTen),
        av::AV_PIX_FMT_P010LE => (Format::I420, Source::SemiPlanarTen),
        _ => return None,
    };
    Some(Image {
        width: width as u32,
        height: height as u32,
        format,
        source,
    })
}

impl Image {
    /// Whether the frame is written the way the decoder laid it out, with no
    /// conversion beyond dropping the padding a stride leaves at each row.
    pub fn passes_through(self) -> bool {
        matches!(self.source, Source::AsIs)
    }

    /// How many bytes `write` fills. Both layouts carry twelve bits a pixel.
    pub fn size(self) -> usize {
        self.width as usize * self.height as usize * 3 / 2
    }

    /// Lay the frame out in `out`, which has to be exactly `size` bytes.
    ///
    /// The buffer is the one the camera is read from, so the frame is written
    /// where it is wanted rather than somewhere it has to be copied from again.
    pub fn write(self, frame: &ffmpeg::Frame, out: &mut [u8]) -> bool {
        if out.len() != self.size() {
            return false;
        }
        let (width, height) = (self.width as usize, self.height as usize);
        let (chroma_width, chroma_height) = (width / 2, height / 2);
        let luma = Grid::direct(width, height);
        let chroma = Grid::direct(chroma_width, chroma_height);
        let out = &mut Out::new(out);
        match self.source {
            // The chroma of an NV12 frame is one plane of half as many rows,
            // holding a pair of samples where I420 holds one.
            Source::AsIs => planar(out, frame, [(0, &luma, 0), (1, &Grid::direct(width, chroma_height), 0)]),
            Source::Planar => planar(out, frame, [(0, &luma, 0), (1, &chroma, 0), (2, &chroma, 0)]),
            Source::PlanarTall => {
                let chroma = Grid { row_step: 2, ..chroma };
                planar(out, frame, [(0, &luma, 0), (1, &chroma, 0), (2, &chroma, 0)])
            }
            Source::PlanarWide => {
                let chroma = Grid {
                    column_step: 2,
                    row_step: 2,
                    ..chroma
                };
                planar(out, frame, [(0, &luma, 0), (1, &chroma, 0), (2, &chroma, 0)])
            }
            Source::PlanarTen => planar(out, frame, [(0, &luma, 2), (1, &chroma, 2), (2, &chroma, 2)]),
            Source::SemiPlanar => planar(out, frame, [(0, &luma, 0)]) && interleaved(out, frame, &chroma, 0, [0, 1]),
            Source::SemiPlanarSwapped => {
                planar(out, frame, [(0, &luma, 0)]) && interleaved(out, frame, &chroma, 0, [1, 0])
            }
            Source::SemiPlanarTen => planar(out, frame, [(0, &luma, 8)]) && interleaved(out, frame, &chroma, 8, [0, 1]),
        }
    }
}

fn planar<const N: usize>(out: &mut Out<'_>, frame: &ffmpeg::Frame, planes: [(usize, &Grid, u32); N]) -> bool {
    planes
        .into_iter()
        .all(|(index, grid, shift)| plane(frame, index, grid).is_some_and(|plane| copy(out, &plane, grid, shift)))
}

fn interleaved(out: &mut Out<'_>, frame: &ffmpeg::Frame, grid: &Grid, shift: u32, offsets: [usize; 2]) -> bool {
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

fn copy(out: &mut Out<'_>, plane: &Plane<'_>, grid: &Grid, shift: u32) -> bool {
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
        // Every row of a grid lays down one sample per column.
        let Some(room) = out.take(grid.columns) else {
            return false;
        };
        copy_row(room, source, grid.column_step, shift);
    }
    true
}

/// Lay one row of a plane down in `room`, keeping every `column_step`th sample.
///
/// The steps a frame actually arrives in are matched on rather than handed to
/// `step_by`, which the compiler cannot see through and turns into a loop over
/// one byte at a time. Whole rows then become a `memcpy` and interleaved chroma
/// a vectorised loop, which is an order of magnitude less work per frame.
fn copy_row(room: &mut [u8], source: &[u8], column_step: usize, shift: u32) {
    if shift == 0 {
        match column_step {
            1 => room.copy_from_slice(source),
            2 => {
                // A row of interleaved chroma holds an odd number of samples,
                // so the one that does not make up a pair is the last one.
                let (pairs, last) = source.as_chunks::<2>();
                let (head, tail) = room.split_at_mut(room.len().min(pairs.len()));
                for (byte, pair) in head.iter_mut().zip(pairs) {
                    *byte = pair[0];
                }
                if let (Some(byte), Some(&sample)) = (tail.first_mut(), last.first()) {
                    *byte = sample;
                }
            }
            step => {
                for (byte, sample) in room.iter_mut().zip(source.iter().step_by(step)) {
                    *byte = *sample;
                }
            }
        }
        return;
    }
    let samples = source.as_chunks::<2>().0;
    match column_step {
        1 => {
            for (byte, sample) in room.iter_mut().zip(samples) {
                *byte = narrow(*sample, shift);
            }
        }
        step => {
            for (byte, sample) in room.iter_mut().zip(samples.iter().step_by(step)) {
                *byte = narrow(*sample, shift);
            }
        }
    }
}

/// Narrow one little endian sample of more than eight bits down to eight.
fn narrow(sample: [u8; 2], shift: u32) -> u8 {
    (u16::from_le_bytes(sample) >> shift) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copied(plane: &[u8], stride: usize, grid: &Grid, shift: u32) -> Option<Vec<u8>> {
        let mut data = vec![0u8; grid.columns * grid.rows];
        let mut out = Out::new(&mut data);
        let plane = Plane { data: plane, stride };
        copy(&mut out, &plane, grid, shift).then_some(data)
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
    fn interleaved_chroma_is_kept_whole_when_the_camera_takes_it() {
        // Two rows of U0 V0 U1 V1, with two bytes of stride padding.
        let plane = [10, 20, 11, 21, 99, 99, 12, 22, 13, 23, 99, 99];
        let grid = Grid::direct(4, 2);
        let kept = vec![10, 20, 11, 21, 12, 22, 13, 23];
        assert_eq!(copied(&plane, 6, &grid, 0), Some(kept));
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
