use crate::{HEIGHT, WIDTH};

pub const CHANNELS: usize = 4;
const ALIGNMENT: usize = 256;

#[derive(Debug, PartialEq, Eq)]
pub enum LayoutError {
    EmptyExtent,
    Overflow,
    UnexpectedLayout,
    Size { expected: usize, actual: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub width: usize,
    pub height: usize,
    pub row: usize,
    pub pitch: usize,
    pub staging: usize,
    pub packed: usize,
}

impl Layout {
    pub fn new(width: usize, height: usize) -> Result<Self, LayoutError> {
        if width == 0 || height == 0 {
            return Err(LayoutError::EmptyExtent);
        }
        let row = width.checked_mul(CHANNELS).ok_or(LayoutError::Overflow)?;
        let pitch = row
            .checked_add(ALIGNMENT - 1)
            .ok_or(LayoutError::Overflow)?
            / ALIGNMENT
            * ALIGNMENT;
        let staging = pitch.checked_mul(height).ok_or(LayoutError::Overflow)?;
        let packed = row.checked_mul(height).ok_or(LayoutError::Overflow)?;
        Ok(Self {
            width,
            height,
            row,
            pitch,
            staging,
            packed,
        })
    }

    pub fn fixture() -> Result<Self, LayoutError> {
        let layout = Self::new(WIDTH, HEIGHT)?;
        if (
            layout.width,
            layout.height,
            CHANNELS,
            layout.row,
            layout.pitch,
            layout.staging,
            layout.packed,
        ) != (160, 160, 4, 640, 768, 122_880, 102_400)
        {
            return Err(LayoutError::UnexpectedLayout);
        }
        Ok(layout)
    }

    pub fn check_packed(self, bytes: &[u8]) -> Result<(), LayoutError> {
        check_size(self.packed, bytes.len())
    }

    pub fn repack(self, mapped: &[u8]) -> Result<Vec<u8>, LayoutError> {
        check_size(self.staging, mapped.len())?;
        let mut packed = Vec::with_capacity(self.packed);
        for row in mapped.chunks_exact(self.pitch) {
            packed.extend_from_slice(&row[..self.row]);
        }
        self.check_packed(&packed)?;
        Ok(packed)
    }
}

fn check_size(expected: usize, actual: usize) -> Result<(), LayoutError> {
    if expected != actual {
        return Err(LayoutError::Size { expected, actual });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_pitch_and_padding_removal() {
        let layout = Layout::fixture().unwrap();
        let mut mapped = vec![0xff; layout.staging];
        let mut expected = Vec::new();
        for (y, row) in mapped.chunks_exact_mut(layout.pitch).enumerate() {
            for (x, byte) in row[..layout.row].iter_mut().enumerate() {
                *byte = ((17 * y + x) % 251) as u8;
            }
            expected.extend_from_slice(&row[..layout.row]);
        }
        assert_eq!(layout.repack(&mapped).unwrap(), expected);
        assert_eq!(Layout::new(64, 1).unwrap().pitch, 256);
        assert_eq!(Layout::new(65, 1).unwrap().pitch, 512);
    }

    #[test]
    fn rejects_invalid_dimensions_and_overflow() {
        assert_eq!(Layout::new(0, 1), Err(LayoutError::EmptyExtent));
        assert_eq!(Layout::new(1, 0), Err(LayoutError::EmptyExtent));
        assert_eq!(Layout::new(usize::MAX, 1), Err(LayoutError::Overflow));
        assert_eq!(Layout::new(usize::MAX / 4, 1), Err(LayoutError::Overflow));
        assert_eq!(Layout::new(160, usize::MAX), Err(LayoutError::Overflow));
    }

    #[test]
    fn rejects_short_and_oversize_buffers() {
        let layout = Layout::fixture().unwrap();
        for actual in [layout.staging - 1, layout.staging + 1, layout.packed] {
            assert_eq!(
                layout.repack(&vec![0; actual]),
                Err(LayoutError::Size {
                    expected: layout.staging,
                    actual
                })
            );
        }
        assert!(layout.check_packed(&vec![0; layout.packed + 1]).is_err());
    }
}
