//! 盘点用例 ID 注册表（与 `docs/layout.md` 中的布局子集对齐）。

/// 优先覆盖的用例 ID（盘点 T-F / T-S / T-L / T-B / T-V / T-W）。
pub const PRIORITY_CASE_IDS: &[&str] = &[
    "T-F01", "T-F02", "T-F03", "T-F04", "T-F05", "T-F06", "T-F07", "T-F08", "T-F09", "T-F10",
    "T-F11", "T-F12", "T-F13", "T-F14", "T-F15", "T-F16", "T-F17", "T-F18", "T-F19", "T-F20",
    "T-F21", "T-F22", "T-F23", "T-F24", "T-F25", "T-F26", "T-F27", "T-S01", "T-S02", "T-S03",
    "T-S04", "T-S05", "T-S06", "T-S07", "T-S08", "T-S09", "T-S10", "T-S11", "T-S12", "T-S13",
    "T-S14", "T-S15", "T-S16", "T-S17", "T-S18", "T-S19", "T-S20", "T-S21", "T-S22", "T-L01",
    "T-L02", "T-L03", "T-L04", "T-B01", "T-B02", "T-B03", "T-B04", "T-B05", "T-B06", "T-B07",
    "T-B08", "T-B09", "T-B10", "T-B11", "T-B12", "T-B13", "T-V01", "T-V02", "T-D01", "T-W01",
    "T-W02", "T-W03", "T-W04", "T-W05", "T-W06", "T-W07", "T-W08", "T-W09", "T-G01", "T-G02",
    "T-G03", "T-G04", "T-G05", "T-G06", "T-G07", "T-G08", "T-G09", "T-G10", "T-G11", "T-G12",
    "T-G13", "T-G14", "T-G15", "T-G16", "T-G17", "T-G18", "T-G19", "T-G20", "T-G21", "T-G22",
    "T-G23", "T-G24", "T-G25", "T-G26", "T-G27", "T-G28", "T-G29", "T-G30", "T-G31", "T-G32",
    "T-G33", "T-G34", "T-P01", "T-P02", "T-P03", "T-P04", "T-P05", "T-P06", "T-P07", "T-P08",
    "T-P09", "T-P10", "T-P11", "T-P12", "T-P13", "T-P14", "T-P15", "T-P16", "T-P17", "T-P18",
    "T-P19", "T-I01", "T-I02", "T-I03", "T-I04", "T-FL01", "T-FL02", "T-FL03", "T-FL04", "T-FL05",
    "T-FL06",
];

/// `(id, status_pass, gap)` — `gap` 仅仍 ignore 的用例有值。
pub fn catalog() -> &'static [(&'static str, bool, Option<&'static str>)] {
    &[
        ("T-F01", true, None),
        ("T-F02", true, None),
        ("T-F03", true, None),
        ("T-F04", true, None), // JustifySpec::SpaceBetween + measure
        ("T-F05", true, None),
        ("T-F06", true, None),
        ("T-F07", true, None), // child_main_length Column→height Fill
        ("T-F08", true, None),
        ("T-F09", true, None), // space-around
        ("T-F10", true, None), // space-evenly
        ("T-F11", true, None), // gap: row column 双值；Row 主轴=column-gap
        ("T-F12", true, None), // row-gap / column-gap 长手；Column 主轴=row-gap
        ("T-F13", true, None), // row gap% → 相对内容宽
        ("T-F14", true, None), // column gap% → 相对内容高（缺省回退宽）
        ("T-F15", true, None), // justify-content:flex-end
        ("T-F16", true, None), // align-items:flex-end
        ("T-F17", true, None), // flex-grow 加权（flex:1 + flex:2）
        ("T-F18", true, None), // flex-shrink 溢出等比（150+150@200 → 100+100）
        ("T-F19", true, None), // shrink + min-width 冻结（120+80）
        ("T-F20", true, None), // align-self 覆盖 align-items
        ("T-F21", true, None), // flex-direction:row-reverse
        ("T-F22", true, None), // flex order
        ("T-F23", true, None), // align-content center wrap
        ("T-F24", true, None), // align-content space-between wrap
        ("T-F25", true, None), // align-content stretch wrap
        ("T-F26", true, None), // align-content:normal → stretch
        ("T-F27", true, None), // flex:1 <basis> 省略 shrink → CSS 1
        ("T-S01", true, None),
        ("T-S02", true, None), // measure Fill 链
        ("T-S03", true, None),
        ("T-S04", true, None), // 几何：flex:1 + 固定 action；ellipsis 见 css_map 单测
        ("T-S05", true, None),
        ("T-S06", true, None), // max-width clamp
        ("T-S07", true, None), // max-height clamp
        ("T-S08", true, None), // calc(100% - Npx) 轻量子集
        ("T-S09", true, None), // 嵌套 width% + 内层 calc
        ("T-S10", true, None), // calc px+% / %+% / px-px
        ("T-S11", true, None), // 嵌套 px+% padding 父链下 width%
        ("T-S12", true, None), // min-width 非零保底（% 被抬升）
        ("T-S13", true, None), // 多子项 flex min 冻结重分配
        ("T-S14", true, None), // 多子项 flex max 冻结重分配
        ("T-S15", true, None), // vh / min() / calc(100vw-Npx) / clamp
        ("T-S16", true, None), // em / rem / calc(em±px)
        ("T-S17", true, None), // width:max-content vs column stretch
        ("T-S18", true, None), // width:fit-content clamp to available
        ("T-S19", true, None), // width:min-content wrap = widest child
        ("T-S20", true, None), // width:fit-content vs column stretch Fill
        ("T-S21", true, None), // calc * / + nested min/max + var 展开后再算
        ("T-S22", true, None), // 混单位嵌套 min/max 相对包含块兑现
        ("T-L01", true, None),
        ("T-L02", true, None), // 显式 width:220 + flex:1
        ("T-L03", true, None), // class nana-settings-row → space-between
        ("T-L04", true, None), // flex:0 0 220px 无 width（basis 主轴）
        ("T-B01", true, None),
        ("T-B02", true, None), // margin 四值
        ("T-B03", true, None), // margin 两值
        ("T-B04", true, None), // padding 三值
        ("T-B05", true, None), // margin 三值
        ("T-B06", true, None), // % padding + % margin 同链推进兄弟
        ("T-B07", true, None), // column 轴 % margin（相对宽度）推进兄弟
        ("T-B08", true, None), // content-box：100+pad10 → border 120×60；推进 b@x120
        ("T-B09", true, None), // border-box：100+pad10+bw5 → content 70×10；inner@15；b@x100
        ("T-B10", true, None), // 负 margin + rem padding
        ("T-B11", true, None), // logical padding-block/inline → LTR physical
        ("T-B12", true, None), // logical margin-* → LTR physical
        ("T-B13", true, None), // margin:0 auto block center
        ("T-V01", true, None),
        ("T-V02", true, None),  // visibility:hidden → CSS 占位（仍测量）
        ("T-D01", true, None),  // display:contents hoist into flex row
        ("T-W01", true, None),  // flex-wrap measure；Wrap 多行拆分
        ("T-W02", true, None),  // wrap-reverse 行序；borrowed+owned 路径
        ("T-W03", true, None),  // wrap 双值 gap：行内 column-gap / 行间 row-gap
        ("T-W04", true, None),  // wrap 折行计入水平 margin
        ("T-W05", true, None),  // wrap 行间 gap%（auto 高→回退宽）
        ("T-W06", true, None),  // wrap-reverse + 行间 gap%
        ("T-W07", true, None),  // column + flex-wrap 折列（measure）
        ("T-W08", true, None),  // column + wrap-reverse 列序；cd@x0 / ab@x88
        ("T-W09", true, None),  // column-wrap + 垂直 margin 触发折列
        ("T-G01", true, None),  // grid-template-columns 220px 1fr → measure
        ("T-G02", true, None),  // var(...) + minmax(0,1fr) 轻量轨
        ("T-G03", true, None),  // 80px 1fr 1fr 等分剩余
        ("T-G04", true, None),  // 100px 1fr 2fr 权重分轨
        ("T-G05", true, None),  // minmax 非零下限冻结
        ("T-G06", true, None),  // 1fr 1.5fr 小数权重
        ("T-G07", true, None),  // 多轨同时 min 冻结
        ("T-G08", true, None),  // minmax(min, maxPx) 上限
        ("T-G09", true, None),  // 多轨同时 max 冻结
        ("T-G10", true, None),  // grid-template-rows + 双值 gap（row-gap）
        ("T-G11", true, None),  // rows minmax 下限冻结
        ("T-G12", true, None),  // rows minmax 上限钳制
        ("T-G13", true, None),  // rows 多轨同时 min 冻结
        ("T-G14", true, None),  // rows 多轨同时 max 冻结
        ("T-G15", true, None),  // 2D 行轨：未定高子项拉伸到 80px / 第二行 120px
        ("T-G16", true, None),  // columns:none → rows-only
        ("T-G17", true, None),  // rows:none → columns-only
        ("T-G18", true, None),  // 双边 none → 回退 Row
        ("T-G19", true, None),  // grid 轨间隙 gap%
        ("T-G20", true, None),  // repeat(3,1fr) 固定次数
        ("T-G21", true, None),  // 轨 % + 1fr
        ("T-G22", true, None),  // inline-grid 1D 同 grid
        ("T-G23", true, None),  // fit-content(Npx) 上限
        ("T-G24", true, None),  // repeat(2,minmax(240px,1fr)) Repo 诚实轨
        ("T-G25", true, None),  // grid-column span 2
        ("T-G26", true, None),  // auto-flow wrap to second row（4 项 2 列）
        ("T-G27", true, None),  // auto-fit 2 tracks, third child wraps
        ("T-G28", true, None),  // justify-self end in a grid cell
        ("T-G29", true, None),  // grid-template-areas + named grid-area
        ("T-G30", true, None),  // mixed 80px + auto-fit, third wraps
        ("T-G31", true, None),  // named grid lines start/mid/end
        ("T-G32", true, None),  // grid item 100% fills resolved cell
        ("T-G33", true, None),  // nth named line foo 2 / foo
        ("T-G34", true, None),  // auto-fill copies [mid] per expansion; mid 2
        ("T-P01", true, None),  // position:relative + inset
        ("T-P02", true, None),  // absolute 脱流 + top/left
        ("T-P03", true, None),  // absolute right/bottom
        ("T-P04", true, None),  // left+right / top+bottom stretch
        ("T-P05", true, None),  // padded containing block
        ("T-P06", true, None),  // no-inset static origin
        ("T-P07", true, None),  // percent inset
        ("T-P08", true, None),  // nested absolute
        ("T-P09", true, None),  // inset 2-value shorthand stretch
        ("T-P10", true, None),  // inset 混用 % + px
        ("T-P11", true, None),  // 单边混用 % + px stretch
        ("T-P12", true, None),  // inset 三值混用 % + px
        ("T-P13", true, None),  // inset 四值混用 % + px
        ("T-P14", true, None),  // logical inset-block/inline → LTR physical
        ("T-P15", true, None),  // fixed 脱流 + 视口 top/right
        ("T-P16", true, None),  // fixed % inset 相对视口
        ("T-P17", true, None),  // fixed left+right/top+bottom 视口拉伸
        ("T-P18", true, None),  // sticky in-flow unstuck（无 overflow）
        ("T-P19", true, None),  // sticky inside overflow:auto still unstuck at rest
        ("T-I01", true, None),  // inline-block side by side
        ("T-I02", true, None),  // text-align:center on IFC
        ("T-I03", true, None),  // white-space:pre preserves newlines
        ("T-I04", true, None),  // IFC block-level sibling starts a new line
        ("T-FL01", true, None), // float left/right + clear:both
        ("T-FL02", true, None), // two float:left wrap, no overlap
        ("T-FL03", true, None), // clear:both after wrapping same-side floats uses packed bottom
        ("T-FL04", true, None), // float's own clear:left starts below packed left
        ("T-FL05", true, None), // IFC line box shrinks beside float:left; inlines wrap in remainder
        ("T-FL06", true, None), // oversized inline drops below float (no overlap)
    ]
}
