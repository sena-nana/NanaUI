//! 应用动画帧的窗口路由与关闭后迟到事件回归。
use super::*;
use nana_ui_runtime::{AnimationEvent, AnimationEventKind, AnimationFrame, AnimationId};

#[derive(Default)]
struct Animated {
    events: Vec<AnimationEvent>,
}

impl ApplicationState for Animated {
    type Message = ();
    type Error = String;

    fn initialize(_: &RuntimeProgramContext<()>) -> Result<Self, String> {
        unreachable!("constructed directly")
    }

    fn build(
        &mut self,
        _: &mut ApplicationWindow,
        _: &RuntimeProgramContext<()>,
    ) -> Result<(), String> {
        Ok(())
    }

    /// 通过窗口调度需求和返回值同时观测宿主的委托结果。
    fn animation_frame(
        &mut self,
        window: &mut ApplicationWindow,
        frame: AnimationFrame,
        context: &RuntimeProgramContext<()>,
    ) -> Result<RuntimeProgramUpdate, nana_ui_runtime::FrameworkError> {
        self.events.extend(frame.events);
        window.demand = FrameDemand::Continuous(std::num::NonZeroU32::new(60).unwrap());
        Ok(RuntimeProgramUpdate::redraw(context.window_id()))
    }
}

/// 使用进程共享测试设备，保持与现有应用生命周期测试相同的宿主上下文。
fn program_context(id: WindowId) -> RuntimeProgramContext<()> {
    RuntimeProgramContext::new(
        id,
        nana_ui_platform::WindowGeometry::default(),
        crate::test_gpu::context(),
        crate::ResolvedWindowPresentation::closed(),
        crate::CompositionWork::default(),
        Arc::new(|_| {}),
        std::sync::mpsc::sync_channel(1).0,
        None,
        crate::startup::StartupHandle::detached(),
    )
}

/// 每帧只更新目标窗口；窗口关闭后事件保持应用状态与其他窗口原样。
#[test]
fn animation_frames_route_to_the_live_window_and_ignore_closed_windows() {
    let id = WindowId(3);
    let other = WindowId(4);
    let context = program_context(id);
    let mut program = RuntimeApplication {
        state: Animated::default(),
        windows: HashMap::from([
            (id, ApplicationWindow::new()),
            (other, ApplicationWindow::new()),
        ]),
    };
    let event = AnimationEvent {
        id: AnimationId::new(7).unwrap(),
        target: nana_ui_runtime::StableNodeId::new(1).unwrap(),
        kind: AnimationEventKind::Finished,
    };
    let frame = AnimationFrame {
        samples: vec![],
        component_updates: vec![],
        events: vec![event],
        next_deadline: None,
        animation_deadlines_scanned: 1,
        animations_considered: 1,
    };

    let answer =
        RuntimeProgram::animation_frame(&mut program, id, frame.clone(), &context).unwrap();
    assert_eq!(answer.redraw, crate::RuntimeRedraw::Window(id));
    assert_eq!(program.state.events, [event]);
    assert_eq!(
        program.windows[&id].demand,
        FrameDemand::Continuous(std::num::NonZeroU32::new(60).unwrap())
    );
    assert_eq!(program.windows[&other].demand, FrameDemand::OnDemand);

    RuntimeProgram::discard_window(&mut program, id);
    let answer = RuntimeProgram::animation_frame(&mut program, id, frame, &context).unwrap();
    assert_eq!(answer.redraw, crate::RuntimeRedraw::None);
    assert!(answer.window_commands.is_empty());
    assert!(!answer.exit);
    assert_eq!(program.state.events.len(), 1);
    assert_eq!(program.windows[&other].demand, FrameDemand::OnDemand);
}
