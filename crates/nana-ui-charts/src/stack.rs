//! Stacking series that share a `stack` name.

/// Stacks `columns` (one per series, in series order, values by index; NaN
/// is a gap) the way ECharts' `stackStrategy: 'samesign'` does: a positive
/// value sits on the running positive total at its index, a negative one
/// on the running negative total. Returns, per series and index, `(base,
/// top)`; a NaN value gives `(base, NaN)` and adds nothing. Shorter columns
/// are treated as NaN past their end.
pub fn stack_same_sign(columns: &[&[f64]]) -> Vec<Vec<(f64, f64)>> {
    let len = columns.iter().map(|c| c.len()).max().unwrap_or(0);
    // Zero (like ECharts) joins the positive side: it sits on top of the
    // positive stack rather than dropping to the axis.
    let mut positive = vec![0.0; len];
    let mut negative = vec![0.0; len];
    columns
        .iter()
        .map(|column| {
            (0..len)
                .map(|i| match column.get(i).copied().unwrap_or(f64::NAN) {
                    v if v.is_nan() => (positive[i], f64::NAN),
                    v if v >= 0.0 => {
                        let base = positive[i];
                        positive[i] += v;
                        (base, positive[i])
                    }
                    v => {
                        let base = negative[i];
                        negative[i] += v;
                        (base, negative[i])
                    }
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_signs_stack_apart() {
        let a = [10.0, -5.0, 3.0];
        let b = [-4.0, -5.0, 0.0];
        let c = [6.0, 2.0, 1.0];
        let stacked = stack_same_sign(&[&a, &b, &c]);
        assert_eq!(stacked[0], vec![(0.0, 10.0), (0.0, -5.0), (0.0, 3.0)]);
        assert_eq!(stacked[1], vec![(0.0, -4.0), (-5.0, -10.0), (3.0, 3.0)]);
        assert_eq!(stacked[2], vec![(10.0, 16.0), (0.0, 2.0), (3.0, 4.0)]);
    }

    #[test]
    fn gaps_and_ragged_columns() {
        let a = [1.0, f64::NAN, 2.0];
        let b = [3.0, 4.0];
        let c = [5.0, 6.0, 7.0];
        let stacked = stack_same_sign(&[&a, &b, &c]);
        assert!(stacked.iter().all(|s| s.len() == 3));
        assert_eq!(stacked[0][1].0, 0.0);
        assert!(stacked[0][1].1.is_nan());
        assert_eq!(stacked[1][1], (0.0, 4.0));
        assert_eq!(stacked[1][2].0, 2.0);
        assert!(stacked[1][2].1.is_nan());
        assert_eq!(stacked[2], vec![(4.0, 9.0), (4.0, 10.0), (2.0, 9.0)]);
        assert!(stack_same_sign(&[]).is_empty());
    }
}
