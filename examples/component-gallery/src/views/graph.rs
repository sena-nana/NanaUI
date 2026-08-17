use super::*;

impl GalleryState {
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
