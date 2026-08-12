use super::*;
use iced::widget::column;

impl GalleryState {
    pub(super) fn graph_gallery(&self, _colors: Colors) -> Element<'_, GalleryMessage> {
        let tokens = self.theme_tokens();
        let selection = match self.graph_selection.as_ref() {
            Some(GraphSelection::Node(node)) => format!("节点 · {node}"),
            Some(GraphSelection::Port { node, port }) => format!("端口 · {node} / {port}"),
            Some(GraphSelection::Edge(edge)) => format!("连线 · {edge}"),
            None => "未选择".to_owned(),
        };
        let toolbar = row![
            text("节点图").size(14).color(tokens.colors.text),
            text(selection).size(11).color(tokens.colors.muted),
            space::horizontal(),
            button(text("重置视图").size(12))
                .on_press(GalleryMessage::ResetGraphViewport)
                .height(Length::Fixed(UI_METRICS.compact_control_height))
                .padding([0.0, UI_METRICS.compact_control_padding_x])
                .style(button_style(tokens, ButtonKind::Text)),
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .padding([8, 12]);
        let graph = GraphCanvas::new(
            "gallery",
            &self.graph,
            self.graph_viewport,
            self.graph_selection.as_ref(),
            GalleryMessage::Graph,
            tokens,
        )
        .view();
        container(column![toolbar, graph])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(canvas_style(tokens))
            .into()
    }

    pub(super) fn reset_graph_viewport(&mut self) {
        self.graph_viewport = self
            .graph
            .bounds()
            .map(|bounds| GraphViewport::fit(bounds, GraphSize::new(900.0, 560.0), 56.0))
            .unwrap_or_default();
    }
}

pub(super) fn gallery_graph() -> GraphModel {
    let source = GraphNode::new(
        "source",
        "Source",
        GraphPoint::new(20.0, 84.0),
        GraphSize::new(160.0, 96.0),
    )
    .with_port(GraphPort::new(
        "output",
        "Output",
        GraphPortKind::Output,
        GraphPortSide::Right,
    ));
    let transform = GraphNode::new(
        "transform",
        "Transform",
        GraphPoint::new(286.0, 42.0),
        GraphSize::new(184.0, 144.0),
    )
    .with_port(GraphPort::new(
        "input",
        "Input",
        GraphPortKind::Input,
        GraphPortSide::Left,
    ))
    .with_port(GraphPort::new(
        "output",
        "Output",
        GraphPortKind::Output,
        GraphPortSide::Right,
    ));
    let target = GraphNode::new(
        "target",
        "Target",
        GraphPoint::new(574.0, 84.0),
        GraphSize::new(160.0, 96.0),
    )
    .with_port(GraphPort::new(
        "input",
        "Input",
        GraphPortKind::Input,
        GraphPortSide::Left,
    ));
    GraphModel::new(
        vec![source, transform, target],
        vec![
            GraphEdge::new(
                "source-transform",
                GraphEndpoint::new("source", "output"),
                GraphEndpoint::new("transform", "input"),
            ),
            GraphEdge::new(
                "transform-target",
                GraphEndpoint::new("transform", "output"),
                GraphEndpoint::new("target", "input"),
            ),
        ],
    )
    .expect("gallery graph is valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gallery_graph_exposes_interactive_targets() {
        let graph = gallery_graph();
        assert_eq!(graph.nodes().len(), 3);
        assert_eq!(graph.edges().len(), 2);
        assert!(graph.bounds().is_some());
    }
}
