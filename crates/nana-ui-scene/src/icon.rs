use nana_ui_core::Icon;

/// Backend-neutral geometry for Nana's semantic 24x24 line icons.
///
/// Keeping the vector contract in the scene crate lets compatibility painters
/// and native backends consume exactly the same paths without substituting a
/// font glyph whose metrics vary by platform.
#[derive(Debug, Clone, PartialEq)]
pub struct IconGeometry {
    pub shapes: Vec<IconShape>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IconShape {
    Path(Vec<IconPathCommand>),
    Circle {
        center: [f32; 2],
        radius: f32,
    },
    Rect {
        origin: [f32; 2],
        size: [f32; 2],
        filled: bool,
    },
    RoundedRect {
        origin: [f32; 2],
        size: [f32; 2],
        radius: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IconPathCommand {
    MoveTo([f32; 2]),
    LineTo([f32; 2]),
    CubicTo {
        control_a: [f32; 2],
        control_b: [f32; 2],
        to: [f32; 2],
    },
    Close,
}

fn line(from: [f32; 2], to: [f32; 2]) -> IconShape {
    IconShape::Path(vec![
        IconPathCommand::MoveTo(from),
        IconPathCommand::LineTo(to),
    ])
}

fn circle(x: f32, y: f32, radius: f32) -> IconShape {
    IconShape::Circle {
        center: [x, y],
        radius,
    }
}

fn rays(center_radius: f32) -> Vec<IconShape> {
    let mut shapes = vec![circle(12.0, 12.0, center_radius)];
    shapes.extend([
        line([12.0, 2.0], [12.0, 5.0]),
        line([12.0, 19.0], [12.0, 22.0]),
        line([2.0, 12.0], [5.0, 12.0]),
        line([19.0, 12.0], [22.0, 12.0]),
        line([4.9, 4.9], [7.0, 7.0]),
        line([17.0, 17.0], [19.1, 19.1]),
        line([4.9, 19.1], [7.0, 17.0]),
        line([17.0, 7.0], [19.1, 4.9]),
    ]);
    shapes
}

pub fn icon_geometry(icon: Icon) -> IconGeometry {
    use IconPathCommand::{Close, CubicTo, LineTo, MoveTo};

    let shapes = match icon {
        Icon::About => vec![
            circle(12.0, 12.0, 9.0),
            circle(12.0, 8.0, 0.45),
            line([12.0, 11.0], [12.0, 16.0]),
        ],
        Icon::Add => vec![
            IconShape::Rect {
                origin: [10.8, 4.0],
                size: [2.4, 16.0],
                filled: true,
            },
            IconShape::Rect {
                origin: [4.0, 10.8],
                size: [16.0, 2.4],
                filled: true,
            },
        ],
        Icon::Appearance => rays(4.0),
        Icon::ArrowLeft => vec![
            line([20.0, 12.0], [5.0, 12.0]),
            line([11.0, 6.0], [5.0, 12.0]),
            line([5.0, 12.0], [11.0, 18.0]),
        ],
        Icon::ArrowRight => vec![
            line([4.0, 12.0], [19.0, 12.0]),
            line([13.0, 6.0], [19.0, 12.0]),
            line([19.0, 12.0], [13.0, 18.0]),
        ],
        Icon::ChevronDown => vec![IconShape::Path(vec![
            MoveTo([6.0, 9.0]),
            LineTo([12.0, 15.0]),
            LineTo([18.0, 9.0]),
        ])],
        Icon::ChevronRight => vec![IconShape::Path(vec![
            MoveTo([9.0, 6.0]),
            LineTo([15.0, 12.0]),
            LineTo([9.0, 18.0]),
        ])],
        Icon::Chart => vec![
            line([4.0, 20.0], [4.0, 4.0]),
            line([4.0, 20.0], [21.0, 20.0]),
            IconShape::Path(vec![
                MoveTo([6.0, 16.0]),
                LineTo([10.0, 11.0]),
                LineTo([14.0, 14.0]),
                LineTo([20.0, 6.0]),
            ]),
        ],
        Icon::Close => vec![IconShape::Path(vec![
            MoveTo([5.0, 5.0]),
            LineTo([19.0, 19.0]),
            MoveTo([19.0, 5.0]),
            LineTo([5.0, 19.0]),
        ])],
        Icon::Eye => vec![
            IconShape::Path(vec![
                MoveTo([2.5, 12.0]),
                CubicTo {
                    control_a: [6.0, 6.5],
                    control_b: [9.0, 5.0],
                    to: [12.0, 5.0],
                },
                CubicTo {
                    control_a: [15.0, 5.0],
                    control_b: [18.0, 6.5],
                    to: [21.5, 12.0],
                },
                CubicTo {
                    control_a: [18.0, 17.5],
                    control_b: [15.0, 19.0],
                    to: [12.0, 19.0],
                },
                CubicTo {
                    control_a: [9.0, 19.0],
                    control_b: [6.0, 17.5],
                    to: [2.5, 12.0],
                },
            ]),
            circle(12.0, 12.0, 2.5),
        ],
        Icon::File => vec![IconShape::Path(vec![
            MoveTo([6.0, 3.0]),
            LineTo([14.0, 3.0]),
            LineTo([19.0, 8.0]),
            LineTo([19.0, 21.0]),
            LineTo([6.0, 21.0]),
            Close,
            MoveTo([14.0, 3.0]),
            LineTo([14.0, 8.0]),
            LineTo([19.0, 8.0]),
        ])],
        Icon::Folder => vec![IconShape::Path(vec![
            MoveTo([3.0, 6.0]),
            LineTo([9.0, 6.0]),
            LineTo([11.0, 9.0]),
            LineTo([21.0, 9.0]),
            LineTo([21.0, 20.0]),
            LineTo([3.0, 20.0]),
            Close,
        ])],
        Icon::Maximize => vec![IconShape::Rect {
            origin: [5.0, 5.0],
            size: [14.0, 14.0],
            filled: false,
        }],
        Icon::Minimize => vec![line([5.0, 12.0], [19.0, 12.0])],
        Icon::Moon => vec![IconShape::Path(vec![
            MoveTo([17.5, 3.5]),
            CubicTo {
                control_a: [12.7, 4.1],
                control_b: [9.0, 8.0],
                to: [9.0, 12.6],
            },
            CubicTo {
                control_a: [9.0, 17.0],
                control_b: [12.4, 20.2],
                to: [16.8, 20.5],
            },
            CubicTo {
                control_a: [13.8, 22.2],
                control_b: [9.7, 21.6],
                to: [7.0, 18.9],
            },
            CubicTo {
                control_a: [3.0, 14.9],
                control_b: [3.0, 8.5],
                to: [7.0, 4.7],
            },
            CubicTo {
                control_a: [9.9, 2.0],
                control_b: [14.2, 1.6],
                to: [17.5, 3.5],
            },
            Close,
        ])],
        Icon::Nodes => vec![
            circle(6.0, 6.0, 2.0),
            circle(18.0, 12.0, 2.0),
            circle(6.0, 18.0, 2.0),
            line([8.0, 6.8], [16.0, 11.2]),
            line([8.0, 17.2], [16.0, 12.8]),
        ],
        Icon::Restore => vec![IconShape::Path(vec![
            MoveTo([8.0, 5.0]),
            LineTo([19.0, 5.0]),
            LineTo([19.0, 16.0]),
            MoveTo([16.0, 8.0]),
            LineTo([5.0, 8.0]),
            LineTo([5.0, 19.0]),
            LineTo([16.0, 19.0]),
            Close,
        ])],
        Icon::Search => vec![circle(10.5, 10.5, 6.5), line([15.5, 15.5], [21.0, 21.0])],
        Icon::Settings => {
            let mut geometry = rays(7.0);
            geometry.insert(1, circle(12.0, 12.0, 2.6));
            geometry
        }
        Icon::Sidebar | Icon::Workspace => {
            let mut geometry = vec![
                IconShape::RoundedRect {
                    origin: [3.0, 4.0],
                    size: [18.0, 16.0],
                    radius: 2.0,
                },
                line([9.0, 4.0], [9.0, 20.0]),
            ];
            if icon == Icon::Workspace {
                geometry.push(line([9.0, 10.0], [21.0, 10.0]));
            }
            geometry
        }
    };
    IconGeometry { shapes }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_semantic_icon_resolves_to_vector_geometry() {
        for icon in [
            Icon::About,
            Icon::Add,
            Icon::Appearance,
            Icon::ArrowLeft,
            Icon::ArrowRight,
            Icon::ChevronDown,
            Icon::ChevronRight,
            Icon::Chart,
            Icon::Close,
            Icon::Eye,
            Icon::File,
            Icon::Folder,
            Icon::Maximize,
            Icon::Minimize,
            Icon::Moon,
            Icon::Nodes,
            Icon::Restore,
            Icon::Search,
            Icon::Settings,
            Icon::Sidebar,
            Icon::Workspace,
        ] {
            assert!(!icon_geometry(icon).shapes.is_empty());
        }
    }
}
