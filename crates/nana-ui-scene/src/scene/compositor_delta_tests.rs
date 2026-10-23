//! 合成图层内重绘与属性更新的投影、可见性回归。

use super::*;
use crate::{PrimitiveId, SceneRect};

/// 上游将容器透明度交给独立合成组，验收最终呈现时合并图元与组透明度。
fn effective_paint_opacity(scene: &UiScene, primitive: PrimitiveId) -> f32 {
    let paint = scene.draw_primitive(primitive).unwrap().paint_opacity;
    scene
        .primitive_opacity_groups(scene.primitive(primitive).unwrap())
        .iter()
        .fold(paint, |opacity, group| opacity * group.opacity)
}

/// 覆盖稳定动画帧与无关布局更新，保留同一可见性索引。
#[test]
fn repaint_under_scaled_layer_keeps_projection_across_unrelated_layout() {
    check_repainted_projection(1.0);
}

/// 透明度变化使用当前缩放投影，并维持重绘后的可见性边界。
#[test]
fn repaint_under_scaled_layer_keeps_projection_on_opacity_tick() {
    check_repainted_projection(0.5);
}

/// 普通重绘继续复用逻辑投影，无需创建呈现属性缓存。
#[test]
fn repaint_without_layers_keeps_the_retained_projection_fast_path() {
    let mut scene = UiScene::new();
    let mut painted = node(1, None, &[]);
    scene.apply_delta([painted.clone()], []);
    let primitive = PrimitiveId {
        node: id(1),
        slot: 0,
    };
    let before = scene.draw_primitive(primitive).unwrap().transform;
    Arc::make_mut(&mut painted.style).background = Some([0.8, 0.2, 0.3, 1.0]);
    scene.apply_delta([painted], []);
    assert_eq!(scene.draw_primitive(primitive).unwrap().transform, before);
    assert!(scene.draw_attributes.lock().unwrap().is_empty());
}

/// 在真实重绘与增量布局事务间比较绘制投影和视口裁剪结果。
fn check_repainted_projection(opacity_end: f32) {
    let mut scene = UiScene::new();
    let parent = node(1, None, &[2]);
    let mut child = node(2, Some(1), &[]);
    let mut other = node(3, None, &[]);
    other.layout.x = 400.0;
    scene.apply_delta([parent, child.clone(), other.clone()], []);
    let scaled = PaintTransform {
        a: 0.94,
        d: 0.94,
        ..Default::default()
    };
    let mut store = PresentationStore::new();
    store.insert(
        transform_track(1, 1, 0, 400, scaled, scaled),
        MotionValue::Transform(scaled),
    );
    store.insert(
        // 单图元叶节点采样透明度，父容器仅缩放；保留可裁剪源图元的前提。
        // 容器透明度隔离组按上游契约完整绘制源内容，无法用于边界裁剪回归。
        opacity_track(2, 2, 0, 400, 1.0, opacity_end),
        MotionValue::Scalar(1.0),
    );
    scene.apply_presentation(&store, Duration::from_millis(16), None);
    let primitive = PrimitiveId {
        node: id(2),
        slot: 0,
    };
    let presented = scene.draw_primitive(primitive).unwrap().transform;
    // 这个视口位于缩放前后的左边界之间，能检查错误边界造成的裁剪差异。
    let viewport = SceneRect {
        x: 0.0,
        y: 20.0,
        width: 1.0,
        height: 10.0,
    };
    let visible = scene.visible_operations(viewport).unwrap();
    assert!(visible.is_empty());
    let plan = scene.frame_plan().unwrap();
    Arc::make_mut(&mut child.style).background = Some([0.8, 0.2, 0.3, 1.0]);
    scene.apply_delta([child], []);
    assert_eq!(
        scene.draw_primitive(primitive).unwrap().transform,
        presented
    );
    assert_eq!(scene.visible_operations(viewport).unwrap(), visible);
    scene.apply_presentation(&store, Duration::from_millis(32), None);
    assert_eq!(
        scene.draw_primitive(primitive).unwrap().transform,
        presented
    );
    // 无关根节点的布局更新会推进属性版本，当前投影与缓存边界应保持一致。
    other.layout.x += 20.0;
    scene.apply_delta([other], []);
    assert_eq!(
        scene.draw_primitive(primitive).unwrap().transform,
        presented
    );
    assert_eq!(scene.visible_operations(viewport).unwrap(), visible);
    assert!(Arc::ptr_eq(&plan, &scene.frame_plan().unwrap()));
    assert!(scene.visibility.get().is_some());
    scene.audit_retained_projection();
}

/// 首帧和提升等待期由 CPU 呈现整棵子树，重绘后仍使用当前投影。
#[test]
fn pending_promotion_paints_current_opacity_and_transform() {
    let mut scene = UiScene::new();
    let parent = node(1, None, &[2]);
    let mut child = node(2, Some(1), &[]);
    scene.apply_delta([parent, child.clone()], []);
    let primitive = PrimitiveId {
        node: id(2),
        slot: 0,
    };
    let mut store = PresentationStore::new();
    let scaled = PaintTransform {
        a: 0.9,
        d: 0.9,
        ..Default::default()
    };
    store.insert(
        opacity_track(1, 1, 0, 100, 0.0, 1.0),
        MotionValue::Scalar(1.0),
    );
    store.insert(
        transform_track(2, 1, 0, 100, scaled, PaintTransform::default()),
        MotionValue::Transform(PaintTransform::default()),
    );
    scene.apply_presentation(&store, Duration::ZERO, None);
    // 首次呈现会建立容器隔离组；后续采样和重绘继续复用这一结构计划。
    let plan = scene.frame_plan().unwrap();
    assert_eq!(scene.compositor_layer_count(), 0);
    assert_eq!(effective_paint_opacity(&scene, primitive), 0.0);
    assert!((scene.draw_primitive(primitive).unwrap().transform.0[0] - 0.9).abs() < 1e-5);
    assert_eq!(scene.compositor_gpu_motion_ids(id(2)), (0, 0));
    let now = Duration::from_millis(8);
    scene.apply_presentation(&store, now, None);
    let MotionValue::Scalar(expected) = store
        .applied_value(
            MotionTargetId::new(1).unwrap(),
            AnimatableProperty::Opacity,
            now,
        )
        .unwrap()
    else {
        panic!("透明度轨道应提供标量")
    };
    let before = scene.draw_primitive(primitive).unwrap().transform;
    assert_eq!(effective_paint_opacity(&scene, primitive), expected);
    assert_eq!(scene.compositor_layer_count(), 0);
    Arc::make_mut(&mut child.style).background = Some([0.8, 0.2, 0.3, 1.0]);
    scene.apply_delta([child], []);
    assert_eq!(scene.draw_primitive(primitive).unwrap().transform, before);
    assert_eq!(effective_paint_opacity(&scene, primitive), expected);
    assert!(Arc::ptr_eq(&plan, &scene.frame_plan().unwrap()));
    scene.audit_retained_projection();
}

/// 降级等待期保留资源，动画结束和逻辑值更新立即落到实际绘制。
#[test]
fn pending_demotion_paints_logical_rest_and_reuses_layer_for_reopen() {
    let mut scene = UiScene::new();
    let mut surface = node(1, None, &[]);
    scene.apply_delta([surface.clone()], []);
    let primitive = PrimitiveId {
        node: id(1),
        slot: 0,
    };
    let mut store = PresentationStore::new();
    let shifted = PaintTransform {
        e: 40.0,
        ..Default::default()
    };
    store.insert(
        opacity_track(1, 1, 0, 100, 0.0, 1.0),
        MotionValue::Scalar(1.0),
    );
    store.insert(
        transform_track(2, 1, 0, 100, shifted, PaintTransform::default()),
        MotionValue::Transform(PaintTransform::default()),
    );
    scene.apply_presentation(&store, Duration::from_millis(64), None);
    let layer = scene.compositor_layer(id(1)).unwrap();
    let identity = (layer.id, layer.cache_generation);
    assert!(effective_paint_opacity(&scene, primitive) < 1.0);
    store = PresentationStore::new();
    scene.apply_presentation(&store, Duration::from_millis(100), None);
    let layer = scene.compositor_layer(id(1)).unwrap();
    assert_eq!((layer.id, layer.cache_generation), identity);
    assert!(layer.bindings.is_empty());
    assert_eq!(effective_paint_opacity(&scene, primitive), 1.0);
    assert_eq!(
        scene.draw_primitive(primitive).unwrap().transform,
        AffineTransform::IDENTITY
    );
    Arc::make_mut(&mut surface.source_style.layout).opacity = Some(0.6);
    Arc::make_mut(&mut surface.style).opacity = 0.6;
    scene.apply_delta([surface], []);
    scene.apply_presentation(&store, Duration::from_millis(120), None);
    assert_eq!(effective_paint_opacity(&scene, primitive), 0.6);
    store.insert(
        opacity_track(3, 1, 120, 100, 0.6, 1.0),
        MotionValue::Scalar(0.6),
    );
    scene.apply_presentation(&store, Duration::from_millis(120), None);
    assert_eq!(effective_paint_opacity(&scene, primitive), 0.6);
    let layer = scene.compositor_layer(id(1)).unwrap();
    assert_eq!(layer.id, identity.0);
    assert_eq!(layer.bindings.len(), 1);
    scene.audit_retained_projection();
}

/// 提升前取消动画立即清理临时呈现，同时撤下帧唤醒请求。
#[test]
fn cancelling_pending_promotion_restores_logical_presentation() {
    let mut scene = UiScene::new();
    scene.apply_delta([node(1, None, &[])], []);
    let primitive = PrimitiveId {
        node: id(1),
        slot: 0,
    };
    let mut store = PresentationStore::new();
    store.insert(
        opacity_track(1, 1, 0, 100, 0.0, 1.0),
        MotionValue::Scalar(1.0),
    );
    scene.apply_presentation(&store, Duration::ZERO, None);
    assert_eq!(effective_paint_opacity(&scene, primitive), 0.0);
    scene.apply_presentation(&PresentationStore::new(), Duration::from_millis(8), None);
    assert_eq!(effective_paint_opacity(&scene, primitive), 1.0);
    assert!(!scene.compositor_needs_tick());
}
