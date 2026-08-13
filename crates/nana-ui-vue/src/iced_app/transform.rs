/// Paint-only transform wrapper. Layout stays unchanged while drawing and
/// pointer hit testing share the same inverse transformation.
struct PaintTransformWidget<'a, Message> {
    content: Element<'a, Message>,
    transform: crate::css_map::PaintTransform,
}

impl<'a, Message> PaintTransformWidget<'a, Message> {
    fn new(
        content: Element<'a, Message>,
        transform: crate::css_map::PaintTransform,
    ) -> Self {
        Self { content, transform }
    }

    fn matrix(&self, bounds: Rectangle) -> [f32; 6] {
        self.transform
            .around_center(bounds.x, bounds.y, bounds.width, bounds.height)
    }
}

impl<Message> Widget<Message, Theme, Renderer> for PaintTransformWidget<'_, Message> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let matrix = self.matrix(layout.bounds());
        let inverse = inverse_affine(matrix);
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            transform_cursor(cursor, inverse),
            renderer,
            shell,
            &transform_rectangle(*viewport, inverse),
        );
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &adv_renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let matrix = self.matrix(layout.bounds());
        let inverse = inverse_affine(matrix);
        renderer.start_affine_group(layout.bounds(), matrix);
        with_active_paint_affine(matrix, || {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                layout,
                transform_cursor(cursor, inverse),
                &transform_rectangle(*viewport, inverse),
            );
        });
        renderer.end_affine_group();
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let mut operation = AffineOperation {
            inner: operation,
            matrix: self.matrix(layout.bounds()),
        };
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, &mut operation);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let inverse = inverse_affine(self.matrix(layout.bounds()));
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            transform_cursor(cursor, inverse),
            &transform_rectangle(*viewport, inverse),
            renderer,
        )
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: iced::Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let matrix = self.matrix(layout.bounds());
        self.content
            .as_widget_mut()
            .overlay(
                &mut tree.children[0],
                layout,
                renderer,
                viewport,
                translation,
            )
            .map(|content| overlay::Element::new(Box::new(AffineOverlay { content, matrix })))
    }
}

fn inverse_affine([a, b, c, d, e, f]: [f32; 6]) -> Option<[f32; 6]> {
    let determinant = a * d - b * c;
    if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
        return None;
    }
    Some([
        d / determinant,
        -b / determinant,
        -c / determinant,
        a / determinant,
        (c * f - d * e) / determinant,
        (b * e - a * f) / determinant,
    ])
}

fn concat_affine(
    [a, b, c, d, e, f]: [f32; 6],
    [g, h, i, j, k, l]: [f32; 6],
) -> [f32; 6] {
    [
        a * g + c * h,
        b * g + d * h,
        a * i + c * j,
        b * i + d * j,
        a * k + c * l + e,
        b * k + d * l + f,
    ]
}

fn is_identity_affine(matrix: [f32; 6]) -> bool {
    matrix == [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
}

fn with_active_paint_affine<T>(matrix: [f32; 6], draw: impl FnOnce() -> T) -> T {
    struct Reset([f32; 6]);
    impl Drop for Reset {
        fn drop(&mut self) {
            ACTIVE_PAINT_AFFINE.with(|active| {
                active.replace(self.0);
            });
        }
    }
    let previous = ACTIVE_PAINT_AFFINE.with(|active| {
        let previous = *active.borrow();
        active.replace(concat_affine(previous, matrix));
        previous
    });
    let _reset = Reset(previous);
    draw()
}

fn transform_point(point: Point, matrix: Option<[f32; 6]>) -> Option<Point> {
    let [a, b, c, d, e, f] = matrix?;
    Some(Point::new(
        a * point.x + c * point.y + e,
        b * point.x + d * point.y + f,
    ))
}

fn transform_cursor(cursor: mouse::Cursor, matrix: Option<[f32; 6]>) -> mouse::Cursor {
    match cursor {
        mouse::Cursor::Available(point) => transform_point(point, matrix)
            .map(mouse::Cursor::Available)
            .unwrap_or_default(),
        mouse::Cursor::Levitating(point) => transform_point(point, matrix)
            .map(mouse::Cursor::Levitating)
            .unwrap_or_default(),
        mouse::Cursor::Unavailable => mouse::Cursor::Unavailable,
    }
}

fn transform_rectangle(rectangle: Rectangle, matrix: Option<[f32; 6]>) -> Rectangle {
    let Some(matrix) = matrix else {
        return Rectangle::default();
    };
    let corners = [
        Point::new(rectangle.x, rectangle.y),
        Point::new(rectangle.x + rectangle.width, rectangle.y),
        Point::new(rectangle.x, rectangle.y + rectangle.height),
        Point::new(
            rectangle.x + rectangle.width,
            rectangle.y + rectangle.height,
        ),
    ];
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for corner in corners {
        let point = transform_point(corner, Some(matrix)).expect("matrix is present");
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }
    Rectangle::new(
        Point::new(min_x, min_y),
        Size::new((max_x - min_x).max(0.0), (max_y - min_y).max(0.0)),
    )
}

fn transform_vector(vector: iced::Vector, [a, b, c, d, _, _]: [f32; 6]) -> iced::Vector {
    iced::Vector::new(a * vector.x + c * vector.y, b * vector.x + d * vector.y)
}

/// Proxies Iced operations without changing layout. This keeps scroll/focus
/// state attached to the original widgets while AccessKit and geometry queries
/// observe the same transformed AABB as drawing and pointer hit testing.
struct AffineOperation<'a> {
    inner: &'a mut dyn widget::Operation,
    matrix: [f32; 6],
}

impl widget::Operation for AffineOperation<'_> {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn widget::Operation)) {
        let matrix = self.matrix;
        self.inner.traverse(&mut |inner| {
            let mut transformed = AffineOperation { inner, matrix };
            operate(&mut transformed);
        });
    }

    fn container(&mut self, id: Option<&widget::Id>, bounds: Rectangle) {
        self.inner
            .container(id, transform_rectangle(bounds, Some(self.matrix)));
    }

    fn scrollable(
        &mut self,
        id: Option<&widget::Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: iced::Vector,
        state: &mut dyn widget::operation::Scrollable,
    ) {
        self.inner.scrollable(
            id,
            transform_rectangle(bounds, Some(self.matrix)),
            transform_rectangle(content_bounds, Some(self.matrix)),
            transform_vector(translation, self.matrix),
            state,
        );
    }

    fn focusable(
        &mut self,
        id: Option<&widget::Id>,
        bounds: Rectangle,
        state: &mut dyn widget::operation::Focusable,
    ) {
        self.inner.focusable(
            id,
            transform_rectangle(bounds, Some(self.matrix)),
            state,
        );
    }

    fn text_input(
        &mut self,
        id: Option<&widget::Id>,
        bounds: Rectangle,
        state: &mut dyn widget::operation::TextInput,
    ) {
        self.inner.text_input(
            id,
            transform_rectangle(bounds, Some(self.matrix)),
            state,
        );
    }

    fn text(&mut self, id: Option<&widget::Id>, bounds: Rectangle, text: &str) {
        self.inner
            .text(id, transform_rectangle(bounds, Some(self.matrix)), text);
    }

    fn custom(
        &mut self,
        id: Option<&widget::Id>,
        bounds: Rectangle,
        state: &mut dyn std::any::Any,
    ) {
        self.inner.custom(
            id,
            transform_rectangle(bounds, Some(self.matrix)),
            state,
        );
    }

    fn finish(&self) -> widget::operation::Outcome<()> {
        self.inner.finish()
    }
}

impl<'a, Message: 'a> From<PaintTransformWidget<'a, Message>> for Element<'a, Message> {
    fn from(widget: PaintTransformWidget<'a, Message>) -> Self {
        Element::new(widget)
    }
}

struct AffineOverlay<'a, Message> {
    content: overlay::Element<'a, Message, Theme, Renderer>,
    matrix: [f32; 6],
}

impl<Message> overlay::Overlay<Message, Theme, Renderer> for AffineOverlay<'_, Message> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        self.content.as_overlay_mut().layout(renderer, bounds)
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &adv_renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        let inverse = inverse_affine(self.matrix);
        renderer.start_affine_group(layout.bounds(), self.matrix);
        with_active_paint_affine(self.matrix, || {
            self.content.as_overlay().draw(
                renderer,
                theme,
                style,
                layout,
                transform_cursor(cursor, inverse),
            );
        });
        renderer.end_affine_group();
    }

    fn operate(
        &mut self,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let mut operation = AffineOperation {
            inner: operation,
            matrix: self.matrix,
        };
        self.content
            .as_overlay_mut()
            .operate(layout, renderer, &mut operation);
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
    ) {
        self.content.as_overlay_mut().update(
            event,
            layout,
            transform_cursor(cursor, inverse_affine(self.matrix)),
            renderer,
            shell,
        );
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_overlay().mouse_interaction(
            layout,
            transform_cursor(cursor, inverse_affine(self.matrix)),
            renderer,
        )
    }

    fn overlay<'a>(
        &'a mut self,
        layout: Layout<'a>,
        renderer: &Renderer,
    ) -> Option<overlay::Element<'a, Message, Theme, Renderer>> {
        let matrix = self.matrix;
        self.content
            .as_overlay_mut()
            .overlay(layout, renderer)
            .map(|content| overlay::Element::new(Box::new(AffineOverlay { content, matrix })))
    }

    fn index(&self) -> f32 {
        self.content.as_overlay().index()
    }
}

fn apply_paint_transform<'a, Message: 'a>(
    content: Element<'a, Message>,
    transform: Option<crate::css_map::PaintTransform>,
) -> Element<'a, Message> {
    match transform.filter(|transform| !transform.is_identity()) {
        Some(transform) => PaintTransformWidget::new(content, transform).into(),
        None => content,
    }
}

fn apply_opacity<'a, Message: 'a>(
    content: Element<'a, Message>,
    opacity: Option<f32>,
) -> Element<'a, Message> {
    match opacity.filter(|opacity| (*opacity - 1.0).abs() > f32::EPSILON) {
        Some(opacity) => OpacityWidget::new(content, opacity.clamp(0.0, 1.0)).into(),
        None => content,
    }
}

struct OpacityWidget<'a, Message> {
    content: Element<'a, Message>,
    opacity: f32,
}

impl<'a, Message> OpacityWidget<'a, Message> {
    fn new(content: Element<'a, Message>, opacity: f32) -> Self {
        Self { content, opacity }
    }
}

impl<Message> Widget<Message, Theme, Renderer> for OpacityWidget<'_, Message> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            shell,
            viewport,
        );
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &adv_renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds().intersection(viewport).unwrap_or_default();
        renderer.start_opacity_group(bounds, self.opacity);
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
        renderer.end_opacity_group();
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: iced::Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let opacity = self.opacity;
        self.content
            .as_widget_mut()
            .overlay(
                &mut tree.children[0],
                layout,
                renderer,
                viewport,
                translation,
            )
            .map(|content| overlay::Element::new(Box::new(OpacityOverlay { content, opacity })))
    }
}

struct OpacityOverlay<'a, Message> {
    content: overlay::Element<'a, Message, Theme, Renderer>,
    opacity: f32,
}

impl<Message> overlay::Overlay<Message, Theme, Renderer> for OpacityOverlay<'_, Message> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        self.content.as_overlay_mut().layout(renderer, bounds)
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &adv_renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        renderer.start_opacity_group(layout.bounds(), self.opacity);
        self.content
            .as_overlay()
            .draw(renderer, theme, style, layout, cursor);
        renderer.end_opacity_group();
    }

    fn operate(
        &mut self,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content
            .as_overlay_mut()
            .operate(layout, renderer, operation);
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
    ) {
        self.content
            .as_overlay_mut()
            .update(event, layout, cursor, renderer, shell);
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_overlay()
            .mouse_interaction(layout, cursor, renderer)
    }

    fn overlay<'a>(
        &'a mut self,
        layout: Layout<'a>,
        renderer: &Renderer,
    ) -> Option<overlay::Element<'a, Message, Theme, Renderer>> {
        let opacity = self.opacity;
        self.content
            .as_overlay_mut()
            .overlay(layout, renderer)
            .map(|content| overlay::Element::new(Box::new(OpacityOverlay { content, opacity })))
    }

    fn index(&self) -> f32 {
        self.content.as_overlay().index()
    }
}

impl<'a, Message: 'a> From<OpacityWidget<'a, Message>> for Element<'a, Message> {
    fn from(widget: OpacityWidget<'a, Message>) -> Self {
        Element::new(widget)
    }
}
