use std::ops::Range;

pub fn should_restart(bytes: u64, review_due: bool) -> bool {
    bytes > 450_000_000 || review_due && bytes > 400_000_000
}

#[derive(Debug, PartialEq, Eq)]
pub struct Slot {
    pub bounds: Range<i32>,
    pub safe: Range<i32>,
}

pub struct Placement {
    pub width: i32,
    pub minimum: i32,
    pub badge: i32,
    pub gutter: i32,
    pub align: i32,
    pub offset: i32,
    pub percent: Option<f64>,
}

pub fn pick(bar: Range<i32>, occupied: &[Range<i32>], p: Placement) -> Option<Slot> {
    if bar.is_empty() {
        return None;
    }
    let mut hits = occupied.to_vec();
    hits.sort_unstable_by_key(|r| r.start);
    let mut cursor = bar.start + p.gutter;
    let mut gaps = Vec::new();
    for hit in hits {
        let left = (hit.start - p.gutter).clamp(bar.start, bar.end);
        if cursor < left {
            gaps.push(cursor..left);
        }
        cursor = cursor.max((hit.end + p.gutter).clamp(bar.start, bar.end));
    }
    if cursor < bar.end - p.gutter {
        gaps.push(cursor..bar.end - p.gutter);
    }
    let target = p.percent.map_or_else(
        || match p.align {
            0 => bar.start,
            1 => (bar.start + bar.end) / 2,
            _ => bar.end,
        },
        |value| bar.start + ((bar.end - bar.start) as f64 * value / 100.).round() as i32,
    );
    let choose = |min| {
        gaps.iter()
            .filter(|r| r.end - r.start >= min)
            .min_by_key(|r| {
                let short =
                    p.percent.is_none() && f64::from(r.end - r.start) < f64::from(p.width) * 0.85;
                (
                    short,
                    0.max(r.start - target).max(target - r.end),
                    r.start - r.end,
                )
            })
            .map(|gap| (gap, min))
    };
    let (gap, min) = choose(p.minimum).or_else(|| choose(p.badge))?;
    let max = p.width.min(gap.end - gap.start);
    let width = if p.percent.is_some() {
        (2 * (target - gap.start).min(gap.end - target)).clamp(min.min(max), max)
    } else {
        max
    };
    let ideal = if p.percent.is_some() {
        target - width / 2
    } else {
        match p.align {
            0 => gap.start + p.offset,
            1 => (gap.start + gap.end - width) / 2 + p.offset,
            _ => gap.end - width - p.offset,
        }
    };
    let x = ideal.clamp(gap.start, gap.end - width);
    Some(Slot {
        bounds: x..x + width,
        safe: gap.clone(),
    })
}
