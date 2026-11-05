//! A survey grid: parallel lines of waypoints that fill a polygon, as Mission Planner's Auto WP,
//! Grid does. The lines run along the first side of the polygon and are `spacing` metres apart; each
//! line goes from where it enters the polygon to where it leaves it, and the next one comes back.
//! The polygon has no holes: a U-shaped one is flown across its open middle too.

/// The waypoints (north, east) of the grid, two to a line, in the order they are flown.
pub fn grid(polygon: &[(f32, f32)], spacing: f32) -> Vec<(f32, f32)> {
    if polygon.len() < 3 || spacing <= 0.0 {
        return vec![];
    }
    let (dn, de) = (polygon[1].0 - polygon[0].0, polygon[1].1 - polygon[0].1);
    let side = (dn * dn + de * de).sqrt();
    if side < 1e-3 {
        return vec![];
    }
    // `along` is the way of the lines, `across` the way from one line to the next.
    let (along, across) = ((dn / side, de / side), (-de / side, dn / side));
    let at = |p: (f32, f32)| (p.0 * along.0 + p.1 * along.1, p.0 * across.0 + p.1 * across.1);
    let corners: Vec<(f32, f32)> = polygon.iter().map(|&p| at(p)).collect();
    let back = |a: f32, b: f32| (a * along.0 + b * across.0, a * along.1 + b * across.1);
    let lowest = corners.iter().map(|c| c.1).fold(f32::MAX, f32::min);
    let highest = corners.iter().map(|c| c.1).fold(f32::MIN, f32::max);
    let mut out = vec![];
    let (mut line, mut b) = (0, lowest + spacing / 2.0);
    while b <= highest {
        // Where this line meets the sides of the polygon: it is inside between the first and the last.
        let (mut first, mut last) = (f32::MAX, f32::MIN);
        for i in 0..corners.len() {
            let ((a0, b0), (a1, b1)) = (corners[i], corners[(i + 1) % corners.len()]);
            if (b0 <= b && b < b1) || (b1 <= b && b < b0) {
                let a = a0 + (b - b0) / (b1 - b0) * (a1 - a0);
                first = first.min(a);
                last = last.max(a);
            }
        }
        if first <= last {
            let (from, to) = if line % 2 == 0 { (first, last) } else { (last, first) };
            out.push(back(from, b));
            out.push(back(to, b));
            line += 1;
        }
        b += spacing;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rectangle_is_filled_with_lines_that_come_and_go() {
        // 100 m east and 90 m north, with the lines 30 m apart: three lines.
        let g = grid(&[(0.0, 0.0), (0.0, 100.0), (90.0, 100.0), (90.0, 0.0)], 30.0);
        assert_eq!(g.len(), 6);
        // The first line runs east, the second back west, and each stays inside the rectangle.
        assert!(g[0].1 < g[1].1 && g[2].1 > g[3].1 && g[4].1 < g[5].1);
        assert!(g.iter().all(|&(n, e)| (0.0..=90.0).contains(&n) && (-0.01..=100.01).contains(&e)));
    }

    #[test]
    fn a_polygon_with_fewer_than_three_corners_has_no_grid() {
        assert!(grid(&[(0.0, 0.0), (10.0, 10.0)], 10.0).is_empty());
        assert!(grid(&[(0.0, 0.0), (0.0, 0.0), (10.0, 10.0)], 10.0).is_empty());
    }

    #[test]
    fn a_u_shape_is_flown_across_its_open_middle() {
        // A U opening to the north: the lines in its arms run through the gap.
        let u = [
            (0.0, 0.0),
            (0.0, 90.0),
            (60.0, 90.0),
            (60.0, 60.0),
            (20.0, 60.0),
            (20.0, 30.0),
            (60.0, 30.0),
            (60.0, 0.0),
        ];
        let g = grid(&u, 10.0);
        // A line at 40 m north has no polygon edges beyond the gap's ends, but spans the whole width.
        assert!(g.chunks(2).any(|l| (l[0].1 - l[1].1).abs() > 85.0 && (l[0].0 - 45.0).abs() < 1.0));
    }
}
