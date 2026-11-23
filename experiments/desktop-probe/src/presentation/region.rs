use super::{ClientExtent, Point, ValidationError};

/// Covered pixels on row `y` from `start_x` up to, excluding, `end_x`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowSpan {
    pub y: i32,
    pub start_x: i32,
    pub end_x: i32,
}

/// Client-local hit coverage stored as sorted maximal spans on each row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HitRegion {
    extent: ClientExtent,
    spans: Vec<RowSpan>,
}

impl HitRegion {
    /// Takes spans in ascending `(y, start_x)` order. Adjacent spans on the
    /// same row coalesce. Overlap, empty spans and out-of-bounds coordinates
    /// are errors. An empty iterator represents empty coverage.
    pub fn new(
        extent: ClientExtent,
        spans: impl IntoIterator<Item = RowSpan>,
    ) -> Result<Self, ValidationError> {
        let mut canonical: Vec<RowSpan> = Vec::new();
        for span in spans {
            if span.start_x >= span.end_x {
                return Err(ValidationError::EmptySpan);
            }
            if !extent.contains(Point::new(span.start_x, span.y)) || span.end_x > extent.right() {
                return Err(ValidationError::SpanOutOfBounds);
            }
            if let Some(previous) = canonical.last_mut() {
                if (span.y, span.start_x) < (previous.y, previous.start_x) {
                    return Err(ValidationError::UnsortedSpans);
                }
                if span.y == previous.y {
                    if span.start_x < previous.end_x {
                        return Err(ValidationError::OverlappingSpans);
                    }
                    if span.start_x == previous.end_x {
                        previous.end_x = span.end_x;
                        continue;
                    }
                }
            }
            canonical.push(span);
        }
        Ok(Self {
            extent,
            spans: canonical,
        })
    }

    pub fn extent(&self) -> ClientExtent {
        self.extent
    }

    pub fn spans(&self) -> &[RowSpan] {
        &self.spans
    }

    pub fn contains(&self, point: Point) -> bool {
        let preceding = self
            .spans
            .partition_point(|span| (span.y, span.start_x) <= (point.y, point.x));
        preceding.checked_sub(1).is_some_and(|index| {
            let span = self.spans[index];
            span.y == point.y && point.x < span.end_x
        })
    }
}
