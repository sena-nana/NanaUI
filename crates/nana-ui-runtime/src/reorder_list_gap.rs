// 拖动让位的共享几何：从稳定布局移除源行，在目标位置插入同高度空槽。

struct ReorderGap {
    offsets: Vec<f32>,
    top: f32,
}

impl ReorderDragVisual {
    /// 各行相对稳定布局的目标纵向位移；源行返回零，其预览继续跟随指针。
    /// 调用方用同一组稳定布局盒驱动动画，避免活动呈现值改变落点判定。
    pub fn gap_offsets(&self, rows: &[LayoutBox], spacing: f32) -> Option<Vec<f32>> {
        self.gap_geometry(rows, spacing).map(|gap| gap.offsets)
    }

    /// 落点和空槽共用同一几何，插入线标记空槽顶边。
    fn gap_drop_mark(
        &self,
        bounds: LayoutBox,
        rows: &[LayoutBox],
        spacing: f32,
    ) -> Option<ReorderDropMark> {
        let gap = self.gap_geometry(rows, spacing)?;
        if self.rules.flat_drop(rows, self.source, self.y).is_none() {
            return None;
        }
        Some(ReorderDropMark::Line(LayoutBox {
            x: bounds.x + INSERT_INSET,
            y: gap.top - INSERT_THICKNESS,
            width: (bounds.width - INSERT_INSET * 2.0).max(0.0),
            height: INSERT_THICKNESS,
        }))
    }

    /// 排除源行后仍按原布局中点确定 before，与松手时发布的顺序意图一致。
    /// 重新累加剩余行高度及间距，在 before 之前预留源高度加一个行距。
    fn gap_geometry(&self, rows: &[LayoutBox], spacing: f32) -> Option<ReorderGap> {
        self.gap_spacing?;
        if self.tree_drop
            || self.rules.subtree_end(self.source) != self.source
            || !spacing.is_finite()
            || !self.y.is_finite()
            || rows
                .iter()
                .any(|row| !row.y.is_finite() || !row.height.is_finite() || row.height < 0.0)
        {
            return None;
        }
        let source = rows.get(self.source)?;
        let spacing = spacing.max(0.0);
        let before = rows.iter().enumerate().find_map(|(index, row)| {
            (index != self.source
                && self.rules.rows.get(index).is_some_and(|row| row.target)
                && self.y < row.y + row.height * 0.5)
                .then_some(index)
        });
        let mut cursor = rows.first()?.y;
        let mut top = None;
        let mut offsets = vec![0.0; rows.len()];
        for (index, row) in rows.iter().enumerate() {
            if index == self.source {
                continue;
            }
            if Some(index) == before {
                top = Some(cursor);
                cursor += source.height + spacing;
            }
            offsets[index] = cursor - row.y;
            cursor += row.height + spacing;
        }
        Some(ReorderGap {
            offsets,
            top: top.unwrap_or(cursor),
        })
    }
}
