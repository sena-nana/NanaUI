// Copyright 2022 The AccessKit Authors. All rights reserved.
// Licensed under the Apache License, Version 2.0 (found in
// the LICENSE-APACHE file) or the MIT license (found in
// the LICENSE-MIT file), at your option.

// Derived from Chromium's accessibility abstraction.
// Copyright 2021 The Chromium Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE.chromium file.

#![allow(non_upper_case_globals)]

use accesskit::{
    Action, ActionData, ActionRequest, AriaCurrent, HasPopup, Invalid, Live, NodeId, Orientation,
    Point, Role, ScrollUnit, SortDirection, Toggled, TreeId,
};
use accesskit_consumer::{FilterResult, FullNodeId, NodeRef, Tree, TreeState};
use std::{
    fmt::Write,
    sync::{Arc, Weak, atomic::Ordering},
};
use windows::{
    Win32::{
        Foundation::*,
        System::{Com::*, Variant::*},
        UI::Accessibility::*,
    },
    core::*,
};

use crate::{
    context::Context,
    filters::{filter, filter_with_root_exception},
    text::PlatformRange as PlatformTextRange,
    util::*,
};

const RUNTIME_ID_SIZE: usize = 5;

fn axis_has_scroll_range(min: Option<f64>, max: Option<f64>) -> bool {
    matches!(
        (min, max),
        (Some(min), Some(max)) if min.is_finite() && max.is_finite() && max > min
    )
}

/// `UIA_ScrollPatternNoScroll` means "no position on this axis".
fn scroll_percent(current: Option<f64>, min: Option<f64>, max: Option<f64>) -> f64 {
    let (Some(current), Some(min), Some(max)) = (current, min, max) else {
        return UIA_ScrollPatternNoScroll;
    };
    if !axis_has_scroll_range(Some(min), Some(max)) || !current.is_finite() {
        return UIA_ScrollPatternNoScroll;
    }
    (100.0 * (current - min) / (max - min)).clamp(0.0, 100.0)
}

/// View size is the viewport as a percentage of the content.
///
/// AccessKit stores the viewport as the node's own bounds and the hidden
/// remainder as `scroll_*_max - scroll_*_min`. There is no separate viewport
/// field. When that pair is missing, report a fully visible view instead of
/// inventing a content size.
fn scroll_view_size(
    viewport: Option<f64>,
    min: Option<f64>,
    max: Option<f64>,
    scrollable: bool,
) -> f64 {
    if !scrollable {
        return 100.0;
    }
    let (Some(viewport), Some(min), Some(max)) = (viewport, min, max) else {
        return 100.0;
    };
    if !(viewport.is_finite() && viewport >= 0.0 && axis_has_scroll_range(Some(min), Some(max))) {
        return 100.0;
    }
    let total = viewport + (max - min);
    if total <= 0.0 {
        return 100.0;
    }
    (100.0 * viewport / total).clamp(0.0, 100.0)
}

fn offset_for_scroll_percent(
    current: Option<f64>,
    min: Option<f64>,
    max: Option<f64>,
    percent: f64,
) -> Result<f64> {
    if percent == UIA_ScrollPatternNoScroll {
        return Ok(current.unwrap_or(0.0));
    }
    if !(percent.is_finite() && (0.0..=100.0).contains(&percent)) {
        return Err(invalid_arg());
    }
    if !axis_has_scroll_range(min, max) {
        return Err(not_supported());
    }
    let min = min.unwrap();
    let max = max.unwrap();
    Ok(min + (percent / 100.0) * (max - min))
}

/// Map a UIA `ScrollAmount` onto AccessKit's existing directional scroll actions.
/// Increment moves toward larger scroll offsets. `NoAmount` changes nothing.
fn uia_scroll_action(
    amount: ScrollAmount,
    horizontal: bool,
) -> Result<Option<(Action, ScrollUnit)>> {
    let mapped = match (amount, horizontal) {
        (ScrollAmount_NoAmount, _) => None,
        (ScrollAmount_SmallDecrement, true) => Some((Action::ScrollLeft, ScrollUnit::Item)),
        (ScrollAmount_LargeDecrement, true) => Some((Action::ScrollLeft, ScrollUnit::Page)),
        (ScrollAmount_SmallIncrement, true) => Some((Action::ScrollRight, ScrollUnit::Item)),
        (ScrollAmount_LargeIncrement, true) => Some((Action::ScrollRight, ScrollUnit::Page)),
        (ScrollAmount_SmallDecrement, false) => Some((Action::ScrollUp, ScrollUnit::Item)),
        (ScrollAmount_LargeDecrement, false) => Some((Action::ScrollUp, ScrollUnit::Page)),
        (ScrollAmount_SmallIncrement, false) => Some((Action::ScrollDown, ScrollUnit::Item)),
        (ScrollAmount_LargeIncrement, false) => Some((Action::ScrollDown, ScrollUnit::Page)),
        _ => return Err(invalid_arg()),
    };
    Ok(mapped)
}

fn directed_scroll(
    node: &NodeRef<'_>,
    amount: ScrollAmount,
    horizontal: bool,
) -> Result<Option<(Action, ScrollUnit)>> {
    let Some((action, unit)) = uia_scroll_action(amount, horizontal)? else {
        return Ok(None);
    };
    if !node.supports_action(action, &filter) {
        return Err(not_supported());
    }
    Ok(Some((action, unit)))
}

fn runtime_id_from_node_id(id: FullNodeId) -> [i32; RUNTIME_ID_SIZE] {
    static_assertions::assert_eq_size!(FullNodeId, u128);
    let id: u128 = id.into();
    [
        UiaAppendRuntimeId as _,
        ((id >> 96) & 0xFFFFFFFF) as _,
        ((id >> 64) & 0xFFFFFFFF) as _,
        ((id >> 32) & 0xFFFFFFFF) as _,
        (id & 0xFFFFFFFF) as _,
    ]
}

pub(crate) struct NodeWrapper<'a> {
    pub(crate) node: &'a NodeRef<'a>,
    pub(crate) string_buffer: &'a mut Vec<u16>,
}

impl NodeWrapper<'_> {
    fn control_type(&self) -> UIA_CONTROLTYPE_ID {
        let role = self.node.role();
        // TODO: Handle special cases. (#14)
        match role {
            Role::Unknown => UIA_CustomControlTypeId,
            Role::TextRun => UIA_CustomControlTypeId,
            Role::Cell | Role::GridCell => UIA_DataItemControlTypeId,
            Role::Label => UIA_TextControlTypeId,
            Role::Image => UIA_ImageControlTypeId,
            Role::Link => UIA_HyperlinkControlTypeId,
            Role::Row => UIA_DataItemControlTypeId,
            Role::ListItem => UIA_ListItemControlTypeId,
            Role::ListMarker => UIA_GroupControlTypeId,
            Role::TreeItem => UIA_TreeItemControlTypeId,
            Role::ListBoxOption => UIA_ListItemControlTypeId,
            Role::MenuItem => UIA_MenuItemControlTypeId,
            Role::MenuListOption => UIA_ListItemControlTypeId,
            Role::Paragraph => UIA_GroupControlTypeId,
            Role::GenericContainer => UIA_GroupControlTypeId,
            Role::CheckBox => UIA_CheckBoxControlTypeId,
            Role::RadioButton => UIA_RadioButtonControlTypeId,
            Role::TextInput
            | Role::MultilineTextInput
            | Role::SearchInput
            | Role::DateInput
            | Role::DateTimeInput
            | Role::WeekInput
            | Role::MonthInput
            | Role::TimeInput
            | Role::EmailInput
            | Role::NumberInput
            | Role::PasswordInput
            | Role::PhoneNumberInput
            | Role::UrlInput => UIA_EditControlTypeId,
            Role::Button | Role::DefaultButton => UIA_ButtonControlTypeId,
            Role::Pane => UIA_PaneControlTypeId,
            Role::RowHeader => UIA_DataItemControlTypeId,
            Role::ColumnHeader => UIA_DataItemControlTypeId,
            Role::RowGroup => UIA_GroupControlTypeId,
            Role::List => UIA_ListControlTypeId,
            Role::Table => UIA_TableControlTypeId,
            Role::LayoutTableCell => UIA_DataItemControlTypeId,
            Role::LayoutTableRow => UIA_DataItemControlTypeId,
            Role::LayoutTable => UIA_TableControlTypeId,
            Role::Switch => UIA_ButtonControlTypeId,
            Role::Menu => UIA_MenuControlTypeId,
            Role::Abbr => UIA_TextControlTypeId,
            Role::Alert => UIA_TextControlTypeId,
            Role::AlertDialog => {
                // Documentation suggests the use of UIA_PaneControlTypeId,
                // but Chromium's implementation uses UIA_WindowControlTypeId
                // instead.
                UIA_WindowControlTypeId
            }
            Role::Application => UIA_PaneControlTypeId,
            Role::Article => UIA_GroupControlTypeId,
            Role::Audio => UIA_GroupControlTypeId,
            Role::Banner => UIA_GroupControlTypeId,
            Role::Blockquote => UIA_GroupControlTypeId,
            Role::Canvas => UIA_ImageControlTypeId,
            Role::Caption => UIA_TextControlTypeId,
            Role::Caret => UIA_GroupControlTypeId,
            Role::Code => UIA_TextControlTypeId,
            Role::ColorWell => UIA_ButtonControlTypeId,
            Role::ComboBox | Role::EditableComboBox => UIA_ComboBoxControlTypeId,
            Role::Complementary => UIA_GroupControlTypeId,
            Role::Comment => UIA_GroupControlTypeId,
            Role::ContentDeletion => UIA_GroupControlTypeId,
            Role::ContentInsertion => UIA_GroupControlTypeId,
            Role::ContentInfo => UIA_GroupControlTypeId,
            Role::Definition => UIA_GroupControlTypeId,
            Role::DescriptionList => UIA_ListControlTypeId,
            Role::Details => UIA_GroupControlTypeId,
            Role::Dialog => {
                // Documentation suggests the use of UIA_PaneControlTypeId,
                // but Chromium's implementation uses UIA_WindowControlTypeId
                // instead.
                UIA_WindowControlTypeId
            }
            Role::DisclosureTriangle => UIA_ButtonControlTypeId,
            Role::Document | Role::Terminal => UIA_DocumentControlTypeId,
            Role::EmbeddedObject => UIA_PaneControlTypeId,
            Role::Emphasis => UIA_TextControlTypeId,
            Role::Feed => UIA_GroupControlTypeId,
            Role::FigureCaption => UIA_TextControlTypeId,
            Role::Figure => UIA_GroupControlTypeId,
            Role::Footer => UIA_GroupControlTypeId,
            Role::Form => UIA_GroupControlTypeId,
            Role::Grid => UIA_DataGridControlTypeId,
            Role::Group => UIA_GroupControlTypeId,
            Role::Header => UIA_GroupControlTypeId,
            Role::Heading => UIA_TextControlTypeId,
            Role::Iframe => UIA_DocumentControlTypeId,
            Role::IframePresentational => UIA_GroupControlTypeId,
            Role::ImeCandidate => UIA_PaneControlTypeId,
            Role::Keyboard => UIA_PaneControlTypeId,
            Role::Legend => UIA_TextControlTypeId,
            Role::LineBreak => UIA_TextControlTypeId,
            Role::ListBox => UIA_ListControlTypeId,
            Role::Log => UIA_GroupControlTypeId,
            Role::Main => UIA_GroupControlTypeId,
            Role::Mark => UIA_TextControlTypeId,
            Role::Marquee => UIA_TextControlTypeId,
            Role::Math => UIA_GroupControlTypeId,
            Role::MenuBar => UIA_MenuBarControlTypeId,
            Role::MenuItemCheckBox => UIA_CheckBoxControlTypeId,
            Role::MenuItemRadio => UIA_RadioButtonControlTypeId,
            Role::MenuListPopup => UIA_ListControlTypeId,
            Role::Meter => UIA_ProgressBarControlTypeId,
            Role::Navigation => UIA_GroupControlTypeId,
            Role::Note => UIA_GroupControlTypeId,
            Role::PluginObject => UIA_GroupControlTypeId,
            Role::ProgressIndicator => UIA_ProgressBarControlTypeId,
            Role::RadioGroup => UIA_GroupControlTypeId,
            Role::Region => UIA_GroupControlTypeId,
            Role::RootWebArea => UIA_DocumentControlTypeId,
            Role::Ruby => UIA_GroupControlTypeId,
            Role::RubyAnnotation => {
                // Generally exposed as description on <ruby> (Role::Ruby)
                // element, not as its own object in the tree.
                // However, it's possible to make a RubyAnnotation element
                // show up in the AX tree, for example by adding tabindex="0"
                // to the source <rp> or <rt> element or making the source element
                // the target of an aria-owns. Therefore, browser side needs to
                // gracefully handle it if it actually shows up in the tree.
                UIA_TextControlTypeId
            }
            Role::ScrollBar => UIA_ScrollBarControlTypeId,
            Role::ScrollView => UIA_PaneControlTypeId,
            Role::Search => UIA_GroupControlTypeId,
            Role::Section => UIA_GroupControlTypeId,
            Role::SectionFooter => UIA_GroupControlTypeId,
            Role::SectionHeader => UIA_GroupControlTypeId,
            Role::Slider => UIA_SliderControlTypeId,
            Role::SpinButton => UIA_SpinnerControlTypeId,
            Role::Splitter => UIA_SeparatorControlTypeId,
            Role::Status => UIA_StatusBarControlTypeId,
            Role::Strong => UIA_TextControlTypeId,
            Role::Suggestion => UIA_GroupControlTypeId,
            Role::SvgRoot => UIA_ImageControlTypeId,
            Role::Tab => UIA_TabItemControlTypeId,
            Role::TabList => UIA_TabControlTypeId,
            Role::TabPanel => UIA_PaneControlTypeId,
            Role::Term => UIA_ListItemControlTypeId,
            Role::Time => UIA_TextControlTypeId,
            Role::Timer => UIA_PaneControlTypeId,
            Role::TitleBar => UIA_PaneControlTypeId,
            Role::Toolbar => UIA_ToolBarControlTypeId,
            Role::Tooltip => UIA_ToolTipControlTypeId,
            Role::Tree => UIA_TreeControlTypeId,
            Role::TreeGrid => UIA_DataGridControlTypeId,
            Role::Video => UIA_GroupControlTypeId,
            Role::WebView => UIA_DocumentControlTypeId,
            Role::Window => {
                // TODO: determine whether to use Window or Pane.
                // It may be good to use Pane for nested windows,
                // as Chromium does. (#14)
                UIA_WindowControlTypeId
            }
            Role::PdfActionableHighlight => UIA_CustomControlTypeId,
            Role::PdfRoot => UIA_DocumentControlTypeId,
            Role::GraphicsDocument => UIA_DocumentControlTypeId,
            Role::GraphicsObject => UIA_PaneControlTypeId,
            Role::GraphicsSymbol => UIA_ImageControlTypeId,
            Role::DocAbstract => UIA_GroupControlTypeId,
            Role::DocAcknowledgements => UIA_GroupControlTypeId,
            Role::DocAfterword => UIA_GroupControlTypeId,
            Role::DocAppendix => UIA_GroupControlTypeId,
            Role::DocBackLink => UIA_HyperlinkControlTypeId,
            Role::DocBiblioEntry => UIA_ListItemControlTypeId,
            Role::DocBibliography => UIA_GroupControlTypeId,
            Role::DocBiblioRef => UIA_HyperlinkControlTypeId,
            Role::DocChapter => UIA_GroupControlTypeId,
            Role::DocColophon => UIA_GroupControlTypeId,
            Role::DocConclusion => UIA_GroupControlTypeId,
            Role::DocCover => UIA_ImageControlTypeId,
            Role::DocCredit => UIA_GroupControlTypeId,
            Role::DocCredits => UIA_GroupControlTypeId,
            Role::DocDedication => UIA_GroupControlTypeId,
            Role::DocEndnote => UIA_ListItemControlTypeId,
            Role::DocEndnotes => UIA_GroupControlTypeId,
            Role::DocEpigraph => UIA_GroupControlTypeId,
            Role::DocEpilogue => UIA_GroupControlTypeId,
            Role::DocErrata => UIA_GroupControlTypeId,
            Role::DocExample => UIA_GroupControlTypeId,
            Role::DocFootnote => UIA_ListItemControlTypeId,
            Role::DocForeword => UIA_GroupControlTypeId,
            Role::DocGlossary => UIA_GroupControlTypeId,
            Role::DocGlossRef => UIA_HyperlinkControlTypeId,
            Role::DocIndex => UIA_GroupControlTypeId,
            Role::DocIntroduction => UIA_GroupControlTypeId,
            Role::DocNoteRef => UIA_HyperlinkControlTypeId,
            Role::DocNotice => UIA_GroupControlTypeId,
            Role::DocPageBreak => UIA_SeparatorControlTypeId,
            Role::DocPageFooter => UIA_GroupControlTypeId,
            Role::DocPageHeader => UIA_GroupControlTypeId,
            Role::DocPageList => UIA_GroupControlTypeId,
            Role::DocPart => UIA_GroupControlTypeId,
            Role::DocPreface => UIA_GroupControlTypeId,
            Role::DocPrologue => UIA_GroupControlTypeId,
            Role::DocPullquote => UIA_GroupControlTypeId,
            Role::DocQna => UIA_GroupControlTypeId,
            Role::DocSubtitle => UIA_GroupControlTypeId,
            Role::DocTip => UIA_GroupControlTypeId,
            Role::DocToc => UIA_GroupControlTypeId,
            Role::ListGrid => UIA_DataGridControlTypeId,
        }
    }

    fn localized_control_type(&mut self) -> Option<StrWrapper<'_>> {
        // `role_description` is the schema's localized override (ARIA
        // `aria-roledescription`). The adapter has no string catalog of its own.
        if let Some(description) = self.node.role_description() {
            return Some(StrWrapper::new(description, self.string_buffer));
        }
        // Classic UI Automation has no Switch control type, so `control_type`
        // stays Button. Reuse the English token already exposed as the ARIA role.
        match self.node.role() {
            Role::Switch => Some(StrWrapper::new("switch", self.string_buffer)),
            _ => None,
        }
    }

    fn aria_role(&mut self) -> Option<StrWrapper<'_>> {
        match self.node.role() {
            Role::Alert => Some("alert"),
            Role::AlertDialog => Some("alertdialog"),
            Role::Application => Some("application"),
            Role::Article => Some("article"),
            Role::Banner | Role::Header => Some("banner"),
            Role::Button | Role::DefaultButton => Some("button"),
            Role::Blockquote => Some("blockquote"),
            Role::Caption | Role::FigureCaption => Some("caption"),
            Role::Cell => Some("cell"),
            Role::CheckBox => Some("checkbox"),
            Role::Code => Some("code"),
            Role::ColumnHeader => Some("columnheader"),
            Role::ComboBox | Role::EditableComboBox => Some("combobox"),
            Role::Comment => Some("comment"),
            Role::Complementary => Some("complementary"),
            Role::ContentInfo | Role::Footer => Some("contentinfo"),
            Role::Definition => Some("definition"),
            Role::ContentDeletion => Some("deletion"),
            Role::Dialog => Some("dialog"),
            Role::Document
            | Role::Iframe
            | Role::WebView
            | Role::RootWebArea
            | Role::Terminal
            | Role::PdfRoot => Some("document"),
            Role::Emphasis => Some("emphasis"),
            Role::Feed => Some("feed"),
            Role::Figure => Some("figure"),
            Role::Form => Some("form"),
            Role::GenericContainer => Some("generic"),
            Role::GraphicsDocument => Some("graphics-document"),
            Role::GraphicsObject => Some("graphics-object"),
            Role::GraphicsSymbol => Some("graphics-symbol"),
            Role::Grid | Role::ListGrid => Some("grid"),
            Role::GridCell => Some("gridcell"),
            Role::Group
            | Role::Details
            | Role::IframePresentational
            | Role::TitleBar
            | Role::LayoutTable
            | Role::LayoutTableCell
            | Role::LayoutTableRow
            | Role::Audio
            | Role::Video
            | Role::ListMarker
            | Role::EmbeddedObject
            | Role::ImeCandidate => Some("group"),
            Role::Heading => Some("heading"),
            Role::Image | Role::Canvas => Some("img"),
            Role::ContentInsertion => Some("insertion"),
            Role::Link => Some("link"),
            Role::List | Role::DescriptionList | Role::MenuListPopup => Some("list"),
            Role::ListBox => Some("listbox"),
            Role::ListItem => Some("listitem"),
            Role::Log => Some("log"),
            Role::Main => Some("main"),
            Role::Mark => Some("marker"),
            Role::Marquee => Some("marquee"),
            Role::Math => Some("math"),
            Role::Menu => Some("menu"),
            Role::MenuBar => Some("menubar"),
            Role::MenuItem => Some("menuitem"),
            Role::MenuItemCheckBox => Some("menuitemcheckbox"),
            Role::MenuItemRadio => Some("menuitemradio"),
            Role::Meter => Some("meter"),
            Role::Navigation => Some("navigation"),
            Role::Note => Some("note"),
            Role::ListBoxOption | Role::MenuListOption => Some("option"),
            Role::Paragraph => Some("paragraph"),
            Role::ProgressIndicator => Some("progressbar"),
            Role::RadioButton => Some("radio"),
            Role::RadioGroup => Some("radiogroup"),
            Role::Region
            | Role::Pane
            | Role::Window
            | Role::Keyboard
            | Role::Unknown
            | Role::ScrollView
            | Role::Caret => Some("region"),
            Role::Row => Some("row"),
            Role::RowGroup => Some("rowgroup"),
            Role::RowHeader => Some("rowheader"),
            Role::ScrollBar => Some("scrollbar"),
            Role::Search => Some("search"),
            Role::SearchInput => Some("searchbox"),
            Role::SectionFooter => Some("sectionfooter"),
            Role::SectionHeader => Some("sectionheader"),
            Role::Splitter => Some("separator"),
            Role::Slider => Some("slider"),
            Role::SpinButton => Some("spinbutton"),
            Role::Status => Some("status"),
            Role::Strong => Some("strong"),
            // subscript
            Role::Suggestion => Some("suggestion"),
            // superscript
            Role::Switch => Some("switch"),
            Role::Tab => Some("tab"),
            Role::Table => Some("table"),
            Role::TabList => Some("tablist"),
            Role::TabPanel => Some("tabpanel"),
            Role::Term => Some("term"),
            Role::TextInput
            | Role::MultilineTextInput
            | Role::DateInput
            | Role::DateTimeInput
            | Role::WeekInput
            | Role::MonthInput
            | Role::TimeInput
            | Role::EmailInput
            | Role::NumberInput
            | Role::PasswordInput
            | Role::PhoneNumberInput
            | Role::UrlInput
            | Role::ColorWell => Some("textbox"),
            Role::Time => Some("time"),
            Role::Timer => Some("timer"),
            Role::Toolbar => Some("toolbar"),
            Role::Tooltip => Some("tooltip"),
            Role::Tree => Some("tree"),
            Role::TreeGrid => Some("treegrid"),
            Role::TreeItem => Some("treeitem"),
            _ => {
                // TODO: Expose more ARIA roles.
                None
            }
        }
        .map(|s| StrWrapper::new(s, self.string_buffer))
    }

    fn landmark_type(&self) -> Option<UIA_LANDMARKTYPE_ID> {
        match self.node.role() {
            Role::Main => Some(UIA_MainLandmarkTypeId),
            Role::Navigation => Some(UIA_NavigationLandmarkTypeId),
            Role::Search => Some(UIA_SearchLandmarkTypeId),
            Role::Form => Some(UIA_FormLandmarkTypeId),
            Role::Banner
            | Role::Header
            | Role::ContentInfo
            | Role::Footer
            | Role::Complementary
            | Role::Region => Some(UIA_CustomLandmarkTypeId),
            _ => None,
        }
    }

    fn localized_landmark_type(&mut self) -> Option<StrWrapper<'_>> {
        // Standard UIA landmark types (`Main`, `Navigation`, `Search`, `Form`)
        // have OS-localized names provided by UIA itself; custom landmark types
        // supply a `LocalizedLandmarkType` string per Core-AAM / UIA.
        match self.node.role() {
            Role::Banner | Role::Header => Some("banner"),
            Role::ContentInfo | Role::Footer => Some("content information"),
            Role::Complementary => Some("complementary"),
            Role::Region => Some("region"),
            _ => None,
        }
        .map(|s| StrWrapper::new(s, self.string_buffer))
    }

    fn name_may_have_changed(&self, old: &NodeWrapper) -> bool {
        let new_from_value = self.node.label_comes_from_value();
        let old_from_value = old.node.label_comes_from_value();
        if new_from_value != old_from_value {
            return true;
        }
        if new_from_value {
            match (self.node.data().value(), old.node.data().value()) {
                (Some(new_val), Some(old_val)) => new_val != old_val,
                (None, None) => self.node.supports_text_ranges() || old.node.supports_text_ranges(),
                _ => true,
            }
        } else {
            match (self.node.data().label(), old.node.data().label()) {
                (Some(new_label), Some(old_label)) => new_label != old_label,
                (None, None) => {
                    self.node.labelled_by().next().is_some()
                        || old.node.labelled_by().next().is_some()
                }
                _ => true,
            }
        }
    }

    pub(crate) fn name(&mut self) -> Option<WideString<'_>> {
        let mut result = WideString::new(self.string_buffer);
        if self.node.label_comes_from_value() {
            self.node.write_value(&mut result)
        } else {
            self.node.write_label(&mut result)
        }
        .unwrap()
        .then_some(result)
    }

    fn description(&mut self) -> Option<StrWrapper<'_>> {
        self.node
            .data()
            .description()
            .map(|s| StrWrapper::new(s, self.string_buffer))
    }

    fn culture(&self) -> Option<LocaleName<'_>> {
        self.node.language().map(LocaleName)
    }

    fn placeholder(&mut self) -> Option<StrWrapper<'_>> {
        self.node
            .placeholder()
            .map(|s| StrWrapper::new(s, self.string_buffer))
    }

    fn is_content_element(&self) -> bool {
        filter(self.node) == FilterResult::Include
    }

    fn is_data_valid_for_form(&self) -> bool {
        self.node.data().invalid().is_none()
    }

    fn aria_properties_may_have_changed(&self, old: &NodeWrapper) -> bool {
        self.node.is_live_atomic() != old.node.is_live_atomic()
            || self.node.braille_label() != old.node.braille_label()
            || self.node.braille_role_description() != old.node.braille_role_description()
            || self.node.is_busy() != old.node.is_busy()
            || self.node.column_index_text() != old.node.column_index_text()
            || self.node.aria_current() != old.node.aria_current()
            || self.node.has_popup() != old.node.has_popup()
            || self.node.data().invalid() != old.node.data().invalid()
            || self.node.level() != old.node.level()
            || self.node.is_multiline() != old.node.is_multiline()
            || self.node.row_index_text() != old.node.row_index_text()
            || self.node.sort_direction() != old.node.sort_direction()
    }

    fn aria_properties(&mut self) -> Option<WideString<'_>> {
        let mut result = WideString::new(self.string_buffer);
        let mut properties = AriaProperties::new(&mut result);

        // TODO: Atomic, and busy flags should include false when explicitly set to that
        if self.node.is_live_atomic() {
            properties.write_bool_property("atomic", true).unwrap();
        }

        if let Some(label) = self.node.braille_label() {
            properties.write_property("braillelabel", label).unwrap();
        }

        if let Some(description) = self.node.braille_role_description() {
            properties
                .write_property("brailleroledescription", description)
                .unwrap();
        }

        if self.node.is_busy() {
            properties.write_bool_property("busy", true).unwrap();
        }

        if let Some(colindextext) = self.node.column_index_text() {
            properties
                .write_property("colindextext", colindextext)
                .unwrap();
        }

        if let Some(current) = self.node.aria_current() {
            if current != AriaCurrent::False {
                properties
                    .write_property(
                        "current",
                        match current {
                            AriaCurrent::True => "true",
                            AriaCurrent::Page => "page",
                            AriaCurrent::Step => "step",
                            AriaCurrent::Location => "location",
                            AriaCurrent::Date => "date",
                            AriaCurrent::Time => "time",
                            AriaCurrent::False => unreachable!(),
                        },
                    )
                    .unwrap();
            }
        }

        if let Some(has_popup) = self.node.has_popup() {
            properties
                .write_property(
                    "haspopup",
                    match has_popup {
                        HasPopup::Menu => "menu",
                        HasPopup::Listbox => "listbox",
                        HasPopup::Tree => "tree",
                        HasPopup::Grid => "grid",
                        HasPopup::Dialog => "dialog",
                    },
                )
                .unwrap();
        }

        if let Some(invalid) = self.node.data().invalid() {
            properties
                .write_property(
                    "invalid",
                    match invalid {
                        Invalid::True => "true",
                        Invalid::Grammar => "grammar",
                        Invalid::Spelling => "spelling",
                    },
                )
                .unwrap();
        }

        if let Some(level) = self.node.level().and_then(|level| level.checked_add(1)) {
            properties.write_usize_property("level", level).unwrap();
        }

        if self.node.is_multiline() {
            properties.write_bool_property("multiline", true).unwrap();
        }

        if let Some(rowindextext) = self.node.row_index_text() {
            properties
                .write_property("rowindextext", rowindextext)
                .unwrap();
        }

        if let Some(sort_direction) = self.node.sort_direction() {
            properties
                .write_property(
                    "sort",
                    match sort_direction {
                        SortDirection::Ascending => "ascending",
                        SortDirection::Descending => "descending",
                        SortDirection::Other => "other",
                    },
                )
                .unwrap();
        }

        if properties.has_properties() {
            Some(result)
        } else {
            None
        }
    }

    fn is_enabled(&self) -> bool {
        !self.node.is_disabled()
    }

    fn is_focusable(&self) -> bool {
        self.node.is_focusable(&filter)
    }

    fn is_focused(&self) -> bool {
        self.node.is_focused()
    }

    fn live_setting(&self) -> LiveSetting {
        let live = self.node.live();
        match live {
            Live::Off => Off,
            Live::Polite => Polite,
            Live::Assertive => Assertive,
        }
    }

    fn automation_id(&mut self) -> Option<StrWrapper<'_>> {
        self.node
            .html_id()
            .or_else(|| self.node.author_id())
            .map(|s| StrWrapper::new(s, self.string_buffer))
    }

    fn class_name(&mut self) -> Option<StrWrapper<'_>> {
        self.node
            .class_name()
            .map(|s| StrWrapper::new(s, self.string_buffer))
    }

    fn orientation(&self) -> OrientationType {
        match self.node.orientation() {
            Some(Orientation::Horizontal) => OrientationType_Horizontal,
            Some(Orientation::Vertical) => OrientationType_Vertical,
            None => OrientationType_None,
        }
    }

    fn is_toggle_pattern_supported(&self) -> bool {
        self.node.toggled().is_some() && !self.is_selection_item_pattern_supported()
    }

    fn toggle_state(&self) -> ToggleState {
        match self.node.toggled().unwrap_or(Toggled::False) {
            Toggled::False => ToggleState_Off,
            Toggled::True => ToggleState_On,
            Toggled::Mixed => ToggleState_Indeterminate,
        }
    }

    fn is_invoke_pattern_supported(&self) -> bool {
        self.node.is_invocable(&filter)
    }

    fn is_value_pattern_supported(&self) -> bool {
        if self.node.supports_url() {
            return true;
        }
        self.node.has_value() && !self.node.label_comes_from_value()
    }

    fn is_range_value_pattern_supported(&self) -> bool {
        self.node.numeric_value().is_some()
    }

    fn value(&mut self) -> WideString<'_> {
        if let Some(url) = self.node.supports_url().then(|| self.node.url()).flatten() {
            let mut result = WideString::new(self.string_buffer);
            result.write_str(url).unwrap();
            return result;
        }
        let mut result = WideString::new(self.string_buffer);
        self.node.write_value(&mut result).unwrap();
        result
    }

    fn is_read_only(&self) -> bool {
        self.node.is_read_only()
    }

    fn numeric_value(&self) -> f64 {
        self.node.numeric_value().unwrap_or(0.0)
    }

    fn min_numeric_value(&self) -> f64 {
        self.node.min_numeric_value().unwrap_or(0.0)
    }

    fn max_numeric_value(&self) -> f64 {
        self.node.max_numeric_value().unwrap_or(0.0)
    }

    fn numeric_value_step(&self) -> f64 {
        self.node.numeric_value_step().unwrap_or(0.0)
    }

    fn numeric_value_jump(&self) -> f64 {
        self.node
            .numeric_value_jump()
            .unwrap_or_else(|| self.numeric_value_step())
    }

    fn is_required(&self) -> bool {
        self.node.is_required()
    }

    fn is_scroll_item_pattern_supported(&self) -> bool {
        self.node.supports_action(Action::ScrollIntoView, &filter)
    }

    fn horizontally_scrollable(&self) -> bool {
        axis_has_scroll_range(self.node.scroll_x_min(), self.node.scroll_x_max())
            || self.node.supports_action(Action::ScrollLeft, &filter)
            || self.node.supports_action(Action::ScrollRight, &filter)
    }

    fn vertically_scrollable(&self) -> bool {
        axis_has_scroll_range(self.node.scroll_y_min(), self.node.scroll_y_max())
            || self.node.supports_action(Action::ScrollUp, &filter)
            || self.node.supports_action(Action::ScrollDown, &filter)
    }

    fn is_scroll_pattern_supported(&self) -> bool {
        self.horizontally_scrollable() || self.vertically_scrollable()
    }

    fn horizontal_scroll_percent(&self) -> f64 {
        scroll_percent(
            self.node.scroll_x(),
            self.node.scroll_x_min(),
            self.node.scroll_x_max(),
        )
    }

    fn vertical_scroll_percent(&self) -> f64 {
        scroll_percent(
            self.node.scroll_y(),
            self.node.scroll_y_min(),
            self.node.scroll_y_max(),
        )
    }

    fn horizontal_view_size(&self) -> f64 {
        let viewport = self.node.raw_bounds().map(|bounds| bounds.width());
        scroll_view_size(
            viewport,
            self.node.scroll_x_min(),
            self.node.scroll_x_max(),
            self.horizontally_scrollable(),
        )
    }

    fn vertical_view_size(&self) -> f64 {
        let viewport = self.node.raw_bounds().map(|bounds| bounds.height());
        scroll_view_size(
            viewport,
            self.node.scroll_y_min(),
            self.node.scroll_y_max(),
            self.vertically_scrollable(),
        )
    }

    pub(crate) fn is_selection_item_pattern_supported(&self) -> bool {
        match self.node.role() {
            // TODO: tables (#29)
            // https://www.w3.org/TR/core-aam-1.1/#mapping_state-property_table
            // SelectionItem.IsSelected is exposed when aria-checked is True or
            // False, for 'radio' and 'menuitemradio' roles.
            Role::RadioButton | Role::MenuItemRadio => {
                matches!(self.node.toggled(), Some(Toggled::True | Toggled::False))
            }
            // https://www.w3.org/TR/wai-aria-1.1/#aria-selected
            // SelectionItem.IsSelected is exposed when aria-select is True or False.
            Role::ListBoxOption
            | Role::ListItem
            | Role::MenuListOption
            | Role::Tab
            | Role::TreeItem => self.node.is_selected().is_some(),
            Role::GridCell => true,
            _ => false,
        }
    }

    pub(crate) fn is_selected(&self) -> bool {
        match self.node.role() {
            // https://www.w3.org/TR/core-aam-1.1/#mapping_state-property_table
            // SelectionItem.IsSelected is set according to the True or False
            // value of aria-checked for 'radio' and 'menuitemradio' roles.
            Role::RadioButton | Role::MenuItemRadio => self.node.toggled() == Some(Toggled::True),
            // https://www.w3.org/TR/wai-aria-1.1/#aria-selected
            // SelectionItem.IsSelected is set according to the True or False
            // value of aria-selected.
            _ => self.node.is_selected().unwrap_or(false),
        }
    }

    fn position_in_set(&self) -> Option<i32> {
        self.node
            .position_in_set()
            .and_then(|p| p.try_into().ok())
            .map(|p: i32| p + 1)
    }

    fn size_of_set(&self) -> Option<i32> {
        self.node
            .size_of_set_from_container(&filter)
            .and_then(|s| s.try_into().ok())
    }

    fn level(&self) -> Option<i32> {
        self.node
            .level()
            .and_then(|level| level.checked_add(1))
            .and_then(|level| level.try_into().ok())
    }

    fn is_selection_pattern_supported(&self) -> bool {
        self.node.is_container_with_selectable_children()
    }

    fn is_multiselectable(&self) -> bool {
        self.node.is_multiselectable()
    }

    fn is_text_pattern_supported(&self) -> bool {
        self.node.supports_text_ranges()
    }

    fn is_expand_collapse_pattern_supported(&self) -> bool {
        self.node.supports_expand_collapse()
    }

    fn expand_collapse_state(&self) -> ExpandCollapseState {
        match self.node.data().is_expanded() {
            Some(true) => ExpandCollapseState_Expanded,
            Some(false) => ExpandCollapseState_Collapsed,
            // TODO: Handle the menu button case. (#27)
            None => ExpandCollapseState_LeafNode,
        }
    }

    fn is_password(&self) -> bool {
        self.node.role() == Role::PasswordInput
    }

    fn is_dialog(&self) -> bool {
        self.node.is_dialog()
    }

    fn is_window_pattern_supported(&self) -> bool {
        self.node.is_dialog()
    }

    fn is_modal(&self) -> bool {
        self.node.is_modal()
    }

    pub(crate) fn enqueue_property_changes(
        &mut self,
        queue: &mut Vec<QueuedEvent>,
        context: &Arc<Context>,
        element: &IRawElementProviderSimple,
        old: &mut NodeWrapper,
    ) {
        self.enqueue_simple_property_changes(queue, context, element, old);
        self.enqueue_pattern_property_changes(queue, element, old);
        self.enqueue_property_implied_events(queue, element, old);
    }

    fn enqueue_property_implied_events(
        &self,
        queue: &mut Vec<QueuedEvent>,
        element: &IRawElementProviderSimple,
        old: &NodeWrapper,
    ) {
        if self.is_text_pattern_supported()
            && old.is_text_pattern_supported()
            && self.node.raw_text_selection() != old.node.raw_text_selection()
        {
            queue.push(QueuedEvent::Simple {
                element: element.clone(),
                event_id: UIA_Text_TextSelectionChangedEventId,
            });
        }
    }
}

fn enqueue_property_change(
    queue: &mut Vec<QueuedEvent>,
    element: &IRawElementProviderSimple,
    property_id: UIA_PROPERTY_ID,
    old_value: Variant,
    new_value: Variant,
) {
    let old_value: VARIANT = old_value.into();
    let new_value: VARIANT = new_value.into();
    queue.push(QueuedEvent::PropertyChanged {
        element: element.clone(),
        property_id,
        old_value,
        new_value,
    });
}

#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IRawElementProviderFragmentRoot,
    IToggleProvider,
    IInvokeProvider,
    IValueProvider,
    IRangeValueProvider,
    IScrollItemProvider,
    IScrollProvider,
    ISelectionItemProvider,
    ISelectionProvider,
    ITextProvider,
    IExpandCollapseProvider,
    IWindowProvider
)]
pub(crate) struct PlatformNode {
    pub(crate) context: Weak<Context>,
    pub(crate) node_id: Option<FullNodeId>,
}

impl PlatformNode {
    pub(crate) fn new(context: &Arc<Context>, node_id: FullNodeId) -> ComObject<Self> {
        Self {
            context: Arc::downgrade(context),
            node_id: Some(node_id),
        }
        .into_object()
    }

    pub(crate) fn unspecified_root(context: &Arc<Context>) -> ComObject<Self> {
        Self {
            context: Arc::downgrade(context),
            node_id: None,
        }
        .into_object()
    }

    fn upgrade_context(&self) -> Result<Arc<Context>> {
        upgrade(&self.context)
    }

    fn with_tree_state_and_context<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&TreeState, &Arc<Context>) -> Result<T>,
    {
        self.with_tree_and_context(|tree, context| f(tree.state(), context))
    }

    fn with_tree_and_context<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Tree, &Arc<Context>) -> Result<T>,
    {
        let context = self.upgrade_context()?;
        let tree = context.read_tree();
        f(&tree, &context)
    }

    fn node<'a>(&self, tree: &'a Tree) -> Result<NodeRef<'a>> {
        let state = tree.state();
        if let Some(id) = self.node_id {
            if let Some(node) = state.node_by_id(id) {
                Ok(node)
            } else {
                Err(element_not_available())
            }
        } else {
            Ok(state.root())
        }
    }

    fn node_with_location<'a>(&self, tree: &'a Tree) -> Result<(NodeRef<'a>, NodeId, TreeId)> {
        let node = self.node(tree)?;
        let (local_id, tree_id) = tree
            .state()
            .locate_node(node.id())
            .ok_or_else(element_not_available)?;
        Ok((node, local_id, tree_id))
    }

    fn resolve_with_context<F, T>(&self, f: F) -> Result<T>
    where
        for<'a> F: FnOnce(NodeRef<'a>, &Arc<Context>) -> Result<T>,
    {
        self.with_tree_and_context(|tree, context| {
            let node = self.node(tree)?;
            f(node, context)
        })
    }

    fn resolve_with_tree_state_and_context<F, T>(&self, f: F) -> Result<T>
    where
        for<'a> F: FnOnce(NodeRef<'a>, &TreeState, &Arc<Context>) -> Result<T>,
    {
        self.with_tree_and_context(|tree, context| {
            let node = self.node(tree)?;
            f(node, tree.state(), context)
        })
    }

    fn resolve<F, T>(&self, f: F) -> Result<T>
    where
        for<'a> F: FnOnce(NodeRef<'a>) -> Result<T>,
    {
        self.resolve_with_context(|node, _| f(node))
    }

    fn resolve_with_context_for_text_pattern<F, T>(&self, f: F) -> Result<T>
    where
        for<'a> F: FnOnce(NodeRef<'a>, &Arc<Context>) -> Result<T>,
    {
        self.with_tree_and_context(|tree, context| {
            let node = self.node(tree)?;
            if node.supports_text_ranges() {
                f(node, context)
            } else {
                Err(element_not_available())
            }
        })
    }

    fn resolve_for_text_pattern<F, T>(&self, f: F) -> Result<T>
    where
        for<'a> F: FnOnce(NodeRef<'a>) -> Result<T>,
    {
        self.resolve_with_context_for_text_pattern(|node, _| f(node))
    }

    fn do_complex_action<F>(&self, f: F) -> Result<()>
    where
        for<'a> F: FnOnce(NodeRef<'a>, NodeId, TreeId) -> Result<Option<ActionRequest>>,
    {
        let context = self.upgrade_context()?;
        if context.is_placeholder.load(Ordering::SeqCst) {
            return Err(element_not_enabled());
        }
        let tree = context.read_tree();
        let (node, target_node, target_tree) = self.node_with_location(&tree)?;
        if let Some(request) = f(node, target_node, target_tree)? {
            drop(tree);
            context.do_action(request);
        }
        Ok(())
    }

    fn do_action<F>(&self, f: F) -> Result<()>
    where
        F: FnOnce() -> (Action, Option<ActionData>),
    {
        self.do_complex_action(|node, target_node, target_tree| {
            if node.is_disabled() {
                return Err(element_not_enabled());
            }
            let (action, data) = f();
            Ok(Some(ActionRequest {
                action,
                target_tree,
                target_node,
                data,
            }))
        })
    }

    fn click(&self) -> Result<()> {
        self.do_action(|| (Action::Click, None))
    }

    fn scroll(&self, horizontal_amount: ScrollAmount, vertical_amount: ScrollAmount) -> Result<()> {
        let context = self.upgrade_context()?;
        if context.is_placeholder.load(Ordering::SeqCst) {
            return Err(element_not_enabled());
        }
        let tree = context.read_tree();
        let (node, target_node, target_tree) = self.node_with_location(&tree)?;
        if node.is_disabled() {
            return Err(element_not_enabled());
        }
        let requests = [
            directed_scroll(&node, horizontal_amount, true)?,
            directed_scroll(&node, vertical_amount, false)?,
        ];
        drop(tree);
        for (action, unit) in requests.into_iter().flatten() {
            context.do_action(ActionRequest {
                action,
                target_tree,
                target_node,
                data: Some(ActionData::ScrollUnit(unit)),
            });
        }
        Ok(())
    }

    fn set_scroll_percent(&self, horizontal_percent: f64, vertical_percent: f64) -> Result<()> {
        self.do_complex_action(|node, target_node, target_tree| {
            if node.is_disabled() {
                return Err(element_not_enabled());
            }
            if horizontal_percent == UIA_ScrollPatternNoScroll
                && vertical_percent == UIA_ScrollPatternNoScroll
            {
                return Ok(None);
            }
            if !node.supports_action(Action::SetScrollOffset, &filter) {
                return Err(not_supported());
            }
            let x = offset_for_scroll_percent(
                node.scroll_x(),
                node.scroll_x_min(),
                node.scroll_x_max(),
                horizontal_percent,
            )?;
            let y = offset_for_scroll_percent(
                node.scroll_y(),
                node.scroll_y_min(),
                node.scroll_y_max(),
                vertical_percent,
            )?;
            Ok(Some(ActionRequest {
                action: Action::SetScrollOffset,
                target_tree,
                target_node,
                data: Some(ActionData::SetScrollOffset(Point::new(x, y))),
            }))
        })
    }

    fn set_expanded(&self, expanded: bool) -> Result<()> {
        self.do_complex_action(|node, target_node, target_tree| {
            if node.is_disabled() {
                return Err(element_not_enabled());
            }
            let Some(current) = node.data().is_expanded() else {
                return Err(invalid_operation());
            };
            if current == expanded {
                return Err(invalid_operation());
            }
            Ok(Some(ActionRequest {
                action: if expanded {
                    Action::Expand
                } else {
                    Action::Collapse
                },
                target_tree,
                target_node,
                data: None,
            }))
        })
    }

    fn set_selected(&self, selected: bool) -> Result<()> {
        self.do_complex_action(|node, target_node, target_tree| {
            if node.is_disabled() {
                return Err(element_not_enabled());
            }
            let mut buffer = StringBuffer::acquire();
            let wrapper = NodeWrapper {
                node: &node,
                string_buffer: &mut buffer,
            };
            if selected == wrapper.is_selected() {
                return Ok(None);
            }
            Ok(Some(ActionRequest {
                action: Action::Click,
                target_tree,
                target_node,
                data: None,
            }))
        })
    }

    fn is_root(&self, state: &TreeState) -> bool {
        self.node_id.is_some_and(|id| id == state.root_id())
    }
}

#[allow(non_snake_case)]
impl IRawElementProviderSimple_Impl for PlatformNode_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider)
    }

    fn GetPatternProvider(&self, pattern_id: UIA_PATTERN_ID) -> Result<IUnknown> {
        self.pattern_provider(pattern_id)
    }

    fn GetPropertyValue(&self, property_id: UIA_PROPERTY_ID) -> Result<VARIANT> {
        self.resolve_with_tree_state_and_context(|node, state, context| {
            let mut buffer = StringBuffer::acquire();
            let mut wrapper = NodeWrapper {
                node: &node,
                string_buffer: &mut buffer,
            };
            let mut result = wrapper.get_property_value(property_id);
            if result.is_empty() {
                if node.is_root() {
                    match property_id {
                        UIA_NamePropertyId => {
                            result = window_title(context.hwnd, &mut buffer).into();
                        }
                        UIA_NativeWindowHandlePropertyId => {
                            result = (context.hwnd.0.0 as i32).into();
                        }
                        _ => (),
                    }
                }
                match property_id {
                    UIA_FrameworkIdPropertyId => {
                        result = StrWrapper::new(state.toolkit_name(), &mut buffer).into()
                    }
                    UIA_ProviderDescriptionPropertyId => {
                        result = toolkit_description(state, &mut buffer).into()
                    }
                    UIA_ControllerForPropertyId => {
                        let controlled: Vec<IUnknown> = node
                            .controls()
                            .filter(|controlled| filter(controlled) == FilterResult::Include)
                            .map(|controlled| context.get_or_create_platform_node(controlled.id()))
                            .map(|result| result.into_interface::<IRawElementProviderSimple>())
                            .filter_map(|controlled| controlled.cast::<IUnknown>().ok())
                            .collect();
                        result = controlled.into();
                    }
                    _ => (),
                }
            }
            Ok(result.into())
        })
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        self.with_tree_state_and_context(|state, context| {
            if self.is_root(state) {
                unsafe { UiaHostProviderFromHwnd(context.hwnd.0) }
            } else {
                Err(Error::empty())
            }
        })
    }
}

#[allow(non_snake_case)]
impl IRawElementProviderFragment_Impl for PlatformNode_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        self.resolve_with_context(|node, context| {
            let result = match direction {
                NavigateDirection_Parent => node.filtered_parent(&filter_with_root_exception),
                NavigateDirection_NextSibling => node.following_filtered_siblings(&filter).next(),
                NavigateDirection_PreviousSibling => {
                    node.preceding_filtered_siblings(&filter).next()
                }
                NavigateDirection_FirstChild => node.filtered_children(&filter).next(),
                NavigateDirection_LastChild => node.filtered_children(&filter).next_back(),
                _ => None,
            };
            match result {
                Some(result) => Ok(context
                    .get_or_create_platform_node(result.id())
                    .into_interface()),
                None => Err(Error::empty()),
            }
        })
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        let node_id = if let Some(id) = self.node_id {
            id
        } else {
            // Since this `PlatformNode` isn't associated with a specific
            // node ID, but always uses whatever node is currently the root,
            // we shouldn't return a UIA runtime ID calculated from an
            // AccessKit node ID, as we normally do. Fortunately,
            // UIA doesn't seem to actually call `GetRuntimeId` on the root.
            return Err(not_implemented());
        };
        let runtime_id = runtime_id_from_node_id(node_id);
        Ok(safe_array_from_i32_slice(&runtime_id))
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        self.resolve_with_context(|node, context| {
            let rect = match node.bounding_box() {
                Some(rect) => {
                    let client_top_left = context.client_top_left()?;
                    UiaRect {
                        left: rect.x0 + client_top_left.x,
                        top: rect.y0 + client_top_left.y,
                        width: rect.width(),
                        height: rect.height(),
                    }
                }
                None => UiaRect::default(),
            };
            Ok(rect)
        })
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        self.do_action(|| (Action::Focus, None))
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        self.with_tree_state_and_context(|state, context| {
            if self.is_root(state) {
                Ok(self.to_interface())
            } else {
                let root_id = state.root_id();
                Ok(context
                    .get_or_create_platform_node(root_id)
                    .into_interface())
            }
        })
    }
}

#[allow(non_snake_case)]
impl IRawElementProviderFragmentRoot_Impl for PlatformNode_Impl {
    fn ElementProviderFromPoint(&self, x: f64, y: f64) -> Result<IRawElementProviderFragment> {
        self.resolve_with_context(|node, context| {
            let client_top_left = context.client_top_left()?;
            let point = Point::new(x - client_top_left.x, y - client_top_left.y);
            let point = node.transform().inverse() * point;
            node.node_at_point(point, &filter).map_or_else(
                || Err(Error::empty()),
                |node| {
                    Ok(context
                        .get_or_create_platform_node(node.id())
                        .into_interface())
                },
            )
        })
    }

    fn GetFocus(&self) -> Result<IRawElementProviderFragment> {
        self.with_tree_state_and_context(|state, context| {
            if let Some(node) = state.focus() {
                let id = node.id();
                let self_id = if let Some(id) = self.node_id {
                    id
                } else {
                    state.root_id()
                };
                if id != self_id {
                    return Ok(context.get_or_create_platform_node(id).into_interface());
                }
            }
            Err(Error::empty())
        })
    }
}

macro_rules! properties {
    (
        $(($id:ident, $m:ident)),+
        $(; guarded: $(($gid:ident, $gm:ident, $gchanged:ident)),+ $(,)?)?
    ) => {
        impl NodeWrapper<'_> {
            fn get_property_value(&mut self, property_id: UIA_PROPERTY_ID) -> Variant {
                match property_id {
                    $($id => {
                        self.$m().into()
                    })*
                    $($($gid => {
                        self.$gm().into()
                    })*)?
                    _ => Variant::empty()
                }
            }
            fn enqueue_simple_property_changes(
                &mut self,
                queue: &mut Vec<QueuedEvent>,
                context: &Arc<Context>,
                element: &IRawElementProviderSimple,
                old: &mut NodeWrapper,
            ) {
                $({
                    let old_value = old.$m();
                    let new_value = self.$m();
                    if old_value != new_value {
                        enqueue_property_change(
                            queue,
                            element,
                            $id,
                            old_value.into(),
                            new_value.into(),
                        );
                    }
                })*
                $($({
                    if self.$gchanged(old) {
                        let old_value = old.$gm();
                        let new_value = self.$gm();
                        if old_value != new_value {
                            enqueue_property_change(
                                queue,
                                element,
                                $gid,
                                old_value.into(),
                                new_value.into(),
                            );
                        }
                    }
                })*)?

                let controls_changed = !old
                    .node
                    .controls()
                    .filter(|controlled| filter(controlled) == FilterResult::Include)
                    .map(|controlled| controlled.id())
                    .eq(self
                        .node
                        .controls()
                        .filter(|controlled| filter(controlled) == FilterResult::Include)
                        .map(|controlled| controlled.id()));
                if controls_changed {
                    let controls: Vec<IUnknown> = self
                        .node
                        .controls()
                        .filter(|controlled| filter(controlled) == FilterResult::Include)
                        .map(|controlled| {
                            context
                                .get_or_create_platform_node(controlled.id())
                                .into_interface::<IRawElementProviderSimple>()
                                .into()
                        })
                        .collect();
                    enqueue_property_change(
                        queue,
                        element,
                        UIA_ControllerForPropertyId,
                        Variant::empty(),
                        controls.into(),
                    );
                }
            }
        }
    };
}

macro_rules! patterns {
    ($(($pattern_id:ident, $provider_interface:ident, $provider_interface_impl:ident, $is_supported:ident, (
        $(($property_id:ident, $com_getter:ident, $getter:ident, $com_type:ident)),*
    ), (
        $($extra_trait_method:item),*
    ))),+) => {
        impl PlatformNode_Impl {
            fn pattern_provider(&self, pattern_id: UIA_PATTERN_ID) -> Result<IUnknown> {
                self.resolve(|node| {
                    let mut buffer = StringBuffer::acquire();
                    let wrapper = NodeWrapper { node: &node, string_buffer: &mut buffer };
                    match pattern_id {
                        $($pattern_id if wrapper.$is_supported() => {
                            let intermediate: $provider_interface = self.to_interface();
                            return intermediate.cast();
                        })*
                        _ => (),
                    }
                    Err(Error::empty())
                })
            }
        }
        impl NodeWrapper<'_> {
            fn enqueue_pattern_property_changes(
                &mut self,
                queue: &mut Vec<QueuedEvent>,
                element: &IRawElementProviderSimple,
                old: &mut NodeWrapper,
            ) {
                $(if self.$is_supported() && old.$is_supported() {
                    $({
                        let old_value = old.$getter();
                        let new_value = self.$getter();
                        if old_value != new_value {
                            enqueue_property_change(
                                queue,
                                element,
                                $property_id,
                                old_value.into(),
                                new_value.into(),
                            );
                        }
                    })*
                })*
            }
        }
        $(#[allow(non_snake_case)]
        impl $provider_interface_impl for PlatformNode_Impl {
            $(fn $com_getter(&self) -> Result<$com_type> {
                self.resolve(|node| {
                    let mut buffer = StringBuffer::acquire();
                    #[allow(unused_mut)]
                    let mut wrapper = NodeWrapper { node: &node, string_buffer: &mut buffer };
                    if !wrapper.$is_supported() {
                        return Err(not_supported());
                    }
                    Ok(wrapper.$getter().into())
                })
            })*
            $($extra_trait_method)*
        })*
    };
}

properties! {
    (UIA_ControlTypePropertyId, control_type),
    (UIA_LocalizedControlTypePropertyId, localized_control_type),
    (UIA_AriaRolePropertyId, aria_role),
    (UIA_LandmarkTypePropertyId, landmark_type),
    (UIA_LocalizedLandmarkTypePropertyId, localized_landmark_type),
    (UIA_FullDescriptionPropertyId, description),
    (UIA_CulturePropertyId, culture),
    (UIA_HelpTextPropertyId, placeholder),
    (UIA_IsContentElementPropertyId, is_content_element),
    (UIA_IsControlElementPropertyId, is_content_element),
    (UIA_IsDataValidForFormPropertyId, is_data_valid_for_form),
    (UIA_IsEnabledPropertyId, is_enabled),
    (UIA_IsKeyboardFocusablePropertyId, is_focusable),
    (UIA_HasKeyboardFocusPropertyId, is_focused),
    (UIA_LiveSettingPropertyId, live_setting),
    (UIA_AutomationIdPropertyId, automation_id),
    (UIA_ClassNamePropertyId, class_name),
    (UIA_OrientationPropertyId, orientation),
    (UIA_IsRequiredForFormPropertyId, is_required),
    (UIA_IsPasswordPropertyId, is_password),
    (UIA_LevelPropertyId, level),
    (UIA_PositionInSetPropertyId, position_in_set),
    (UIA_SizeOfSetPropertyId, size_of_set),
    (UIA_IsDialogPropertyId, is_dialog);
    guarded:
    (UIA_NamePropertyId, name, name_may_have_changed),
    (UIA_AriaPropertiesPropertyId, aria_properties, aria_properties_may_have_changed)
}

patterns! {
    (UIA_TogglePatternId, IToggleProvider, IToggleProvider_Impl, is_toggle_pattern_supported, (
        (UIA_ToggleToggleStatePropertyId, ToggleState, toggle_state, ToggleState)
    ), (
        fn Toggle(&self) -> Result<()> {
            self.click()
        }
    )),
    (UIA_InvokePatternId, IInvokeProvider, IInvokeProvider_Impl, is_invoke_pattern_supported, (), (
        fn Invoke(&self) -> Result<()> {
            self.click()
        }
    )),
    (UIA_ValuePatternId, IValueProvider, IValueProvider_Impl, is_value_pattern_supported, (
        (UIA_ValueValuePropertyId, Value, value, BSTR),
        (UIA_ValueIsReadOnlyPropertyId, IsReadOnly, is_read_only, BOOL)
    ), (
        fn SetValue(&self, value: &PCWSTR) -> Result<()> {
            let value = unsafe { value.to_string() }.map_err(|_| invalid_arg())?;
            self.do_action(|| (Action::SetValue, Some(ActionData::Value(value.into()))))
        }
    )),
    (UIA_RangeValuePatternId, IRangeValueProvider, IRangeValueProvider_Impl, is_range_value_pattern_supported, (
        (UIA_RangeValueValuePropertyId, Value, numeric_value, f64),
        (UIA_RangeValueIsReadOnlyPropertyId, IsReadOnly, is_read_only, BOOL),
        (UIA_RangeValueMinimumPropertyId, Minimum, min_numeric_value, f64),
        (UIA_RangeValueMaximumPropertyId, Maximum, max_numeric_value, f64),
        (UIA_RangeValueSmallChangePropertyId, SmallChange, numeric_value_step, f64),
        (UIA_RangeValueLargeChangePropertyId, LargeChange, numeric_value_jump, f64)
    ), (
        fn SetValue(&self, value: f64) -> Result<()> {
            self.do_action(|| {
                (Action::SetValue, Some(ActionData::NumericValue(value)))
            })
        }
    )),
    (UIA_ScrollItemPatternId, IScrollItemProvider, IScrollItemProvider_Impl, is_scroll_item_pattern_supported, (), (
        fn ScrollIntoView(&self) -> Result<()> {
            self.do_complex_action(|_node, target_node, target_tree| {
                Ok(Some(ActionRequest {
                    action: Action::ScrollIntoView,
                    target_tree,
                    target_node,
                    data: None,
                }))
            })
        }
    )),
    (UIA_ScrollPatternId, IScrollProvider, IScrollProvider_Impl, is_scroll_pattern_supported, (
        (UIA_ScrollHorizontalScrollPercentPropertyId, HorizontalScrollPercent, horizontal_scroll_percent, f64),
        (UIA_ScrollVerticalScrollPercentPropertyId, VerticalScrollPercent, vertical_scroll_percent, f64),
        (UIA_ScrollHorizontalViewSizePropertyId, HorizontalViewSize, horizontal_view_size, f64),
        (UIA_ScrollVerticalViewSizePropertyId, VerticalViewSize, vertical_view_size, f64),
        (UIA_ScrollHorizontallyScrollablePropertyId, HorizontallyScrollable, horizontally_scrollable, BOOL),
        (UIA_ScrollVerticallyScrollablePropertyId, VerticallyScrollable, vertically_scrollable, BOOL)
    ), (
        fn Scroll(&self, horizontal_amount: ScrollAmount, vertical_amount: ScrollAmount) -> Result<()> {
            self.scroll(horizontal_amount, vertical_amount)
        },

        fn SetScrollPercent(&self, horizontal_percent: f64, vertical_percent: f64) -> Result<()> {
            self.set_scroll_percent(horizontal_percent, vertical_percent)
        }
    )),
    (UIA_SelectionItemPatternId, ISelectionItemProvider, ISelectionItemProvider_Impl, is_selection_item_pattern_supported, (), (
        fn IsSelected(&self) -> Result<BOOL> {
            self.resolve(|node| {
                let mut buffer = StringBuffer::acquire();
                let wrapper = NodeWrapper {
                    node: &node,
                    string_buffer: &mut buffer,
                };
                Ok(wrapper.is_selected().into())
            })
        },

        fn Select(&self) -> Result<()> {
            self.set_selected(true)
        },

        fn AddToSelection(&self) -> Result<()> {
            self.set_selected(true)
        },

        fn RemoveFromSelection(&self) -> Result<()> {
            self.set_selected(false)
        },

        fn SelectionContainer(&self) -> Result<IRawElementProviderSimple> {
            self.resolve_with_context(|node, context| {
                if let Some(container) = node.selection_container(&filter) {
                    Ok(context.get_or_create_platform_node(container.id()).into_interface())
                } else {
                    Err(E_FAIL.into())
                }
            })
        }
    )),
    (UIA_SelectionPatternId, ISelectionProvider, ISelectionProvider_Impl, is_selection_pattern_supported, (
        (UIA_SelectionCanSelectMultiplePropertyId, CanSelectMultiple, is_multiselectable, BOOL),
        (UIA_SelectionIsSelectionRequiredPropertyId, IsSelectionRequired, is_required, BOOL)
    ), (
        fn GetSelection(&self) -> Result<*mut SAFEARRAY> {
            self.resolve_with_context(|node, context| {
                let selection: Vec<_> = node
                    .items(&filter)
                    .filter(|item| item.is_selected() == Some(true))
                    .map(|item| context.get_or_create_platform_node(item.id()).into_interface::<IRawElementProviderSimple>())
                    .filter_map(|item| item.cast::<IUnknown>().ok())
                    .collect();
                Ok(safe_array_from_com_slice(&selection))
            })
        }
    )),
    (UIA_TextPatternId, ITextProvider, ITextProvider_Impl, is_text_pattern_supported, (), (
        fn GetSelection(&self) -> Result<*mut SAFEARRAY> {
            self.resolve_for_text_pattern(|node| {
                if let Some(range) = node.text_selection() {
                    let platform_range: ITextRangeProvider = PlatformTextRange::new(&self.context, range).into();
                    let iunknown: IUnknown = platform_range.cast()?;
                    Ok(safe_array_from_com_slice(&[iunknown]))
                } else {
                    Ok(std::ptr::null_mut())
                }
            })
        },

        fn GetVisibleRanges(&self) -> Result<*mut SAFEARRAY> {
            // TBD: Do we need this? The Quorum GUI toolkit, which is our
            // current point of comparison for text functionality,
            // doesn't implement it.
            Ok(std::ptr::null_mut())
        },

        fn RangeFromChild(&self, _child: Ref<IRawElementProviderSimple>) -> Result<ITextRangeProvider> {
            // We don't support embedded objects in text.
            Err(not_implemented())
        },

        fn RangeFromPoint(&self, point: &UiaPoint) -> Result<ITextRangeProvider> {
            self.resolve_with_context_for_text_pattern(|node, context| {
                let client_top_left = context.client_top_left()?;
                let point = Point::new(point.x - client_top_left.x, point.y - client_top_left.y);
                let point = node.transform().inverse() * point;
                let pos = node.text_position_at_point(point);
                let range = pos.to_degenerate_range();
                Ok(PlatformTextRange::new(&self.context, range).into())
            })
        },

        fn DocumentRange(&self) -> Result<ITextRangeProvider> {
            self.resolve_for_text_pattern(|node| {
                let range = node.document_range();
                Ok(PlatformTextRange::new(&self.context, range).into())
            })
        },

        fn SupportedTextSelection(&self) -> Result<SupportedTextSelection> {
            self.resolve_for_text_pattern(|node| {
                if node.has_text_selection() {
                    Ok(SupportedTextSelection_Single)
                } else {
                    Ok(SupportedTextSelection_None)
                }
            })
        }
    )),
    (UIA_ExpandCollapsePatternId, IExpandCollapseProvider, IExpandCollapseProvider_Impl, is_expand_collapse_pattern_supported, (
        (UIA_ExpandCollapseExpandCollapseStatePropertyId, ExpandCollapseState, expand_collapse_state, ExpandCollapseState)
    ), (
        fn Expand(&self) -> Result<()> {
            self.set_expanded(true)
        },

        fn Collapse(&self) -> Result<()> {
            self.set_expanded(false)
        }
    )),
    (UIA_WindowPatternId, IWindowProvider, IWindowProvider_Impl, is_window_pattern_supported, (
        (UIA_WindowIsModalPropertyId, IsModal, is_modal, BOOL)
    ), (
        fn SetVisualState(&self, _: WindowVisualState) -> Result<()> {
            Err(invalid_operation())
        },

        fn Close(&self) -> Result<()> {
            Err(not_supported())
        },

        fn WaitForInputIdle(&self, _: i32) -> Result<BOOL> {
            Err(not_supported())
        },

        fn CanMaximize(&self) -> Result<BOOL> {
            Err(not_supported())
        },

        fn CanMinimize(&self) -> Result<BOOL> {
            Err(not_supported())
        },

        fn WindowVisualState(&self) -> Result<WindowVisualState> {
            Err(not_supported())
        },

        fn WindowInteractionState(&self) -> Result<WindowInteractionState> {
            Ok(WindowInteractionState_ReadyForUserInteraction)
        },

        fn IsTopmost(&self) -> Result<BOOL> {
            Err(not_supported())
        }
    ))
}

// Ensures that `PlatformNode` is actually safe to use in the free-threaded
// manner that we advertise via `ProviderOptions`.
#[test]
fn platform_node_impl_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<PlatformNode>();
}

#[cfg(test)]
fn child_node(tree: &Tree, id: NodeId) -> NodeRef<'_> {
    tree.state()
        .node_by_tree_local_id(id, TreeId::ROOT)
        .unwrap()
}

#[cfg(test)]
fn localized_eq(actual: Option<StrWrapper<'_>>, expected: &str) -> bool {
    let mut buffer = StringBuffer::acquire();
    actual == Some(StrWrapper::new(expected, &mut buffer))
}

#[cfg(test)]
fn single_child_tree(child_id: NodeId, child: accesskit::Node) -> Tree {
    use accesskit::{TreeInfo, TreeUpdate};

    let mut root = accesskit::Node::new(Role::Window);
    root.set_children(vec![child_id]);
    Tree::new(
        TreeUpdate {
            nodes: vec![(NodeId(0), root), (child_id, child)],
            tree: Some(TreeInfo::new(NodeId(0))),
            tree_id: TreeId::ROOT,
            focus: NodeId(0),
        },
        true,
    )
}

#[test]
fn switch_keeps_button_control_type_and_uses_switch_token() {
    use accesskit::Node;

    let child_id = NodeId(1);
    let tree = single_child_tree(child_id, Node::new(Role::Switch));
    let node = child_node(&tree, child_id);
    let mut buffer = StringBuffer::acquire();
    let mut wrapper = NodeWrapper {
        node: &node,
        string_buffer: &mut buffer,
    };
    assert_eq!(wrapper.control_type(), UIA_ButtonControlTypeId);
    assert!(localized_eq(wrapper.localized_control_type(), "switch"));
}

#[test]
fn switch_role_description_overrides_localized_control_type() {
    use accesskit::Node;

    let child_id = NodeId(1);
    let mut node = Node::new(Role::Switch);
    node.set_role_description("开关");
    let tree = single_child_tree(child_id, node);
    let node = child_node(&tree, child_id);
    let mut buffer = StringBuffer::acquire();
    let mut wrapper = NodeWrapper {
        node: &node,
        string_buffer: &mut buffer,
    };
    assert_eq!(wrapper.control_type(), UIA_ButtonControlTypeId);
    assert!(localized_eq(wrapper.localized_control_type(), "开关"));
}

#[test]
fn button_has_no_synthetic_localized_control_type() {
    use accesskit::Node;

    let child_id = NodeId(1);
    let tree = single_child_tree(child_id, Node::new(Role::Button));
    let node = child_node(&tree, child_id);
    let mut buffer = StringBuffer::acquire();
    let mut wrapper = NodeWrapper {
        node: &node,
        string_buffer: &mut buffer,
    };
    assert!(wrapper.localized_control_type().is_none());
}

#[test]
fn scroll_view_maps_percent_and_viewport_from_existing_fields() {
    use accesskit::{Node, Rect};

    let child_id = NodeId(1);
    let mut node = Node::new(Role::ScrollView);
    node.set_bounds(Rect {
        x0: 0.0,
        y0: 0.0,
        x1: 80.0,
        y1: 100.0,
    });
    node.set_scroll_y(75.0);
    node.set_scroll_y_min(0.0);
    node.set_scroll_y_max(300.0);
    node.add_action(Action::ScrollUp);
    node.add_action(Action::ScrollDown);
    let tree = single_child_tree(child_id, node);
    let node = child_node(&tree, child_id);
    let mut buffer = StringBuffer::acquire();
    let wrapper = NodeWrapper {
        node: &node,
        string_buffer: &mut buffer,
    };
    assert!(wrapper.is_scroll_pattern_supported());
    assert!(!wrapper.horizontally_scrollable());
    assert!(wrapper.vertically_scrollable());
    assert_eq!(
        wrapper.horizontal_scroll_percent(),
        UIA_ScrollPatternNoScroll
    );
    assert_eq!(wrapper.vertical_scroll_percent(), 25.0);
    assert_eq!(wrapper.horizontal_view_size(), 100.0);
    assert_eq!(wrapper.vertical_view_size(), 25.0);
}

#[test]
fn scroll_pattern_follows_scroll_fields_on_other_roles() {
    use accesskit::Node;

    let child_id = NodeId(1);
    let mut node = Node::new(Role::List);
    node.set_scroll_x(0.0);
    node.set_scroll_x_min(0.0);
    node.set_scroll_x_max(10.0);
    node.add_action(Action::ScrollLeft);
    node.add_action(Action::ScrollRight);
    let tree = single_child_tree(child_id, node);
    let node = child_node(&tree, child_id);
    let mut buffer = StringBuffer::acquire();
    let wrapper = NodeWrapper {
        node: &node,
        string_buffer: &mut buffer,
    };
    assert!(wrapper.is_scroll_pattern_supported());
    assert!(wrapper.horizontally_scrollable());
}

#[test]
fn scroll_into_view_alone_does_not_expose_scroll_pattern() {
    use accesskit::Node;

    let child_id = NodeId(1);
    let mut node = Node::new(Role::ListItem);
    node.add_action(Action::ScrollIntoView);
    let tree = single_child_tree(child_id, node);
    let node = child_node(&tree, child_id);
    let mut buffer = StringBuffer::acquire();
    let wrapper = NodeWrapper {
        node: &node,
        string_buffer: &mut buffer,
    };
    assert!(wrapper.is_scroll_item_pattern_supported());
    assert!(!wrapper.is_scroll_pattern_supported());
}

#[test]
fn uia_scroll_amounts_use_existing_scroll_actions() {
    assert_eq!(
        uia_scroll_action(ScrollAmount_NoAmount, false).unwrap(),
        None
    );
    assert_eq!(
        uia_scroll_action(ScrollAmount_SmallIncrement, false).unwrap(),
        Some((Action::ScrollDown, ScrollUnit::Item))
    );
    assert_eq!(
        uia_scroll_action(ScrollAmount_LargeDecrement, true).unwrap(),
        Some((Action::ScrollLeft, ScrollUnit::Page))
    );
    assert_eq!(
        uia_scroll_action(ScrollAmount_LargeIncrement, true).unwrap(),
        Some((Action::ScrollRight, ScrollUnit::Page))
    );
    assert_eq!(
        uia_scroll_action(ScrollAmount_SmallDecrement, false).unwrap(),
        Some((Action::ScrollUp, ScrollUnit::Item))
    );
}

#[test]
fn set_scroll_percent_converts_to_scroll_offset() {
    assert_eq!(
        offset_for_scroll_percent(Some(75.0), Some(0.0), Some(300.0), 50.0).unwrap(),
        150.0
    );
    assert_eq!(
        offset_for_scroll_percent(
            Some(75.0),
            Some(0.0),
            Some(300.0),
            UIA_ScrollPatternNoScroll
        )
        .unwrap(),
        75.0
    );
    assert!(offset_for_scroll_percent(Some(0.0), Some(0.0), Some(10.0), 140.0).is_err());
    assert!(offset_for_scroll_percent(None, None, None, 10.0).is_err());
}

#[cfg(test)]
fn aria_properties_string(node: &NodeRef<'_>) -> Option<String> {
    let mut buffer = StringBuffer::acquire();
    let mut wrapper = NodeWrapper {
        node,
        string_buffer: &mut buffer,
    };
    wrapper
        .aria_properties()
        .map(|ws| String::from_utf16_lossy(ws.as_slice()))
}

#[cfg(test)]
fn property_change_ids(events: &[QueuedEvent]) -> Vec<UIA_PROPERTY_ID> {
    events
        .iter()
        .filter_map(|event| match event {
            QueuedEvent::PropertyChanged { property_id, .. } => Some(*property_id),
            QueuedEvent::Simple { .. } => None,
        })
        .collect()
}

#[test]
fn aria_invalid_maps_to_data_valid_for_form_and_aria_properties() {
    use accesskit::Node;

    let child_id = NodeId(1);

    let tree = single_child_tree(child_id, Node::new(Role::TextInput));
    let node = child_node(&tree, child_id);
    let mut buffer = StringBuffer::acquire();
    let wrapper = NodeWrapper {
        node: &node,
        string_buffer: &mut buffer,
    };
    assert!(wrapper.is_data_valid_for_form());
    assert_eq!(aria_properties_string(&node), None);

    for (invalid, expected) in [
        (Invalid::True, "invalid=true"),
        (Invalid::Grammar, "invalid=grammar"),
        (Invalid::Spelling, "invalid=spelling"),
    ] {
        let mut invalid_node = Node::new(Role::TextInput);
        invalid_node.set_invalid(invalid);
        let tree = single_child_tree(child_id, invalid_node);
        let node = child_node(&tree, child_id);
        let mut buffer = StringBuffer::acquire();
        let wrapper = NodeWrapper {
            node: &node,
            string_buffer: &mut buffer,
        };
        assert!(!wrapper.is_data_valid_for_form());
        assert_eq!(aria_properties_string(&node).as_deref(), Some(expected));
    }
}

#[test]
fn landmark_roles_map_to_uia_landmark_type_and_localized_landmark_type() {
    use accesskit::Node;

    let child_id = NodeId(1);

    for (role, expected_type, expected_localized) in [
        (Role::Main, Some(UIA_MainLandmarkTypeId), None),
        (Role::Navigation, Some(UIA_NavigationLandmarkTypeId), None),
        (Role::Search, Some(UIA_SearchLandmarkTypeId), None),
        (Role::Form, Some(UIA_FormLandmarkTypeId), None),
        (Role::Banner, Some(UIA_CustomLandmarkTypeId), Some("banner")),
        (Role::Header, Some(UIA_CustomLandmarkTypeId), Some("banner")),
        (
            Role::ContentInfo,
            Some(UIA_CustomLandmarkTypeId),
            Some("content information"),
        ),
        (
            Role::Footer,
            Some(UIA_CustomLandmarkTypeId),
            Some("content information"),
        ),
        (
            Role::Complementary,
            Some(UIA_CustomLandmarkTypeId),
            Some("complementary"),
        ),
        (Role::Region, Some(UIA_CustomLandmarkTypeId), Some("region")),
        (Role::Group, None, None),
        (Role::Button, None, None),
        (Role::Pane, None, None),
        (Role::Window, None, None),
    ] {
        let tree = single_child_tree(child_id, Node::new(role));
        let node = child_node(&tree, child_id);
        let mut buffer = StringBuffer::acquire();
        let mut wrapper = NodeWrapper {
            node: &node,
            string_buffer: &mut buffer,
        };
        assert_eq!(wrapper.landmark_type(), expected_type, "role = {role:?}");
        match expected_localized {
            Some(expected) => assert!(
                localized_eq(wrapper.localized_landmark_type(), expected),
                "role = {role:?}"
            ),
            None => assert!(
                wrapper.localized_landmark_type().is_none(),
                "role = {role:?}"
            ),
        }
    }
}

#[test]
fn aria_properties_level_is_one_based_matching_uia_level_property() {
    use accesskit::Node;

    let child_id = NodeId(1);
    for (zero_based, expected_one_based, expected_aria) in
        [(0usize, 1i32, "level=1"), (2usize, 3i32, "level=3")]
    {
        let mut heading = Node::new(Role::Heading);
        heading.set_level(zero_based);
        let tree = single_child_tree(child_id, heading);
        let node = child_node(&tree, child_id);
        let mut buffer = StringBuffer::acquire();
        let wrapper = NodeWrapper {
            node: &node,
            string_buffer: &mut buffer,
        };
        assert_eq!(wrapper.level(), Some(expected_one_based));
        assert_eq!(
            aria_properties_string(&node).as_deref(),
            Some(expected_aria)
        );
    }
}

#[test]
fn controls_diff_skips_platform_node_creation_when_unchanged_and_emits_on_change() {
    use crate::{
        context::{ActionHandlerWrapper, Context},
        window_handle::WindowHandle,
    };
    use accesskit::{ActionHandler, Node, TreeInfo, TreeUpdate};

    struct NullActionHandler;
    impl ActionHandler for NullActionHandler {
        fn do_action(&mut self, _request: ActionRequest) {}
    }

    let make_tree = |controls: Vec<NodeId>| {
        let mut root = Node::new(Role::Window);
        root.set_children(vec![NodeId(1), NodeId(2), NodeId(3)]);
        let mut controller = Node::new(Role::Button);
        controller.set_controls(controls);
        Tree::new(
            TreeUpdate {
                nodes: vec![
                    (NodeId(0), root),
                    (NodeId(1), controller),
                    (NodeId(2), Node::new(Role::Region)),
                    (NodeId(3), Node::new(Role::Region)),
                ],
                tree: Some(TreeInfo::new(NodeId(0))),
                tree_id: TreeId::ROOT,
                focus: NodeId(0),
            },
            true,
        )
    };

    let old_tree = make_tree(vec![NodeId(2)]);
    let same_tree = make_tree(vec![NodeId(2)]);
    let changed_tree = make_tree(vec![NodeId(2), NodeId(3)]);

    let context = Context::new(
        WindowHandle(HWND(std::ptr::null_mut())),
        make_tree(vec![NodeId(2)]),
        Arc::new(ActionHandlerWrapper::new(NullActionHandler)),
        false,
    );
    let controller_full_id = child_node(&same_tree, NodeId(1)).id();
    let element: IRawElementProviderSimple =
        PlatformNode::new(&context, controller_full_id).into_interface();

    // Unchanged controls must neither queue an event nor allocate/cache PlatformNodes.
    let mut queue = Vec::new();
    let old_node = child_node(&old_tree, NodeId(1));
    let same_node = child_node(&same_tree, NodeId(1));
    let mut old_buf = StringBuffer::acquire();
    let mut new_buf = StringBuffer::acquire();
    let mut old_wrapper = NodeWrapper {
        node: &old_node,
        string_buffer: &mut old_buf,
    };
    let mut new_wrapper = NodeWrapper {
        node: &same_node,
        string_buffer: &mut new_buf,
    };
    new_wrapper.enqueue_property_changes(&mut queue, &context, &element, &mut old_wrapper);
    assert!(queue.is_empty());
    assert_eq!(context.platform_node_count(), 0);

    // Changed controls (length increased from 1 to 2) must queue UIA_ControllerForPropertyId
    // and materialize PlatformNodes for the controlled targets.
    let changed_node = child_node(&changed_tree, NodeId(1));
    let mut changed_buf = StringBuffer::acquire();
    let mut changed_wrapper = NodeWrapper {
        node: &changed_node,
        string_buffer: &mut changed_buf,
    };
    changed_wrapper.enqueue_property_changes(&mut queue, &context, &element, &mut old_wrapper);
    assert!(property_change_ids(&queue).contains(&UIA_ControllerForPropertyId));
    assert_eq!(context.platform_node_count(), 2);
}

#[test]
fn guarded_properties_and_invalid_emit_expected_property_changes() {
    use crate::{
        context::{ActionHandlerWrapper, Context},
        window_handle::WindowHandle,
    };
    use accesskit::{ActionHandler, Node, TreeInfo, TreeUpdate};

    struct NullActionHandler;
    impl ActionHandler for NullActionHandler {
        fn do_action(&mut self, _request: ActionRequest) {}
    }

    let mut old_input = Node::new(Role::TextInput);
    old_input.set_label("Email");
    old_input.set_description("Enter your email");
    let old_tree = single_child_tree(NodeId(1), old_input);

    let mut new_input = Node::new(Role::TextInput);
    new_input.set_label("Email");
    new_input.set_description("Enter your email");
    new_input.set_invalid(Invalid::True);
    let new_tree = single_child_tree(NodeId(1), new_input);

    let context = Context::new(
        WindowHandle(HWND(std::ptr::null_mut())),
        single_child_tree(NodeId(1), Node::new(Role::TextInput)),
        Arc::new(ActionHandlerWrapper::new(NullActionHandler)),
        false,
    );
    let node_id = child_node(&new_tree, NodeId(1)).id();
    let element: IRawElementProviderSimple = PlatformNode::new(&context, node_id).into_interface();

    let old_node = child_node(&old_tree, NodeId(1));
    let new_node = child_node(&new_tree, NodeId(1));
    let mut old_buf = StringBuffer::acquire();
    let mut new_buf = StringBuffer::acquire();
    let mut old_wrapper = NodeWrapper {
        node: &old_node,
        string_buffer: &mut old_buf,
    };
    let mut new_wrapper = NodeWrapper {
        node: &new_node,
        string_buffer: &mut new_buf,
    };
    let mut queue = Vec::new();
    new_wrapper.enqueue_property_changes(&mut queue, &context, &element, &mut old_wrapper);
    let props = property_change_ids(&queue);
    assert!(props.contains(&UIA_IsDataValidForFormPropertyId));
    assert!(props.contains(&UIA_AriaPropertiesPropertyId));
    assert!(!props.contains(&UIA_NamePropertyId));
    assert!(!props.contains(&UIA_FullDescriptionPropertyId));

    // Also verify indirect `labelled_by` name changes are still detected through the guard.
    let make_labelled_by_tree = |label_text: &str| {
        let mut root = Node::new(Role::Window);
        root.set_children(vec![NodeId(1), NodeId(2)]);
        let mut target = Node::new(Role::Button);
        target.set_labelled_by(vec![NodeId(2)]);
        let mut label = Node::new(Role::Label);
        label.set_value(label_text);
        Tree::new(
            TreeUpdate {
                nodes: vec![(NodeId(0), root), (NodeId(1), target), (NodeId(2), label)],
                tree: Some(TreeInfo::new(NodeId(0))),
                tree_id: TreeId::ROOT,
                focus: NodeId(0),
            },
            true,
        )
    };
    let t1 = make_labelled_by_tree("Old name");
    let t2 = make_labelled_by_tree("New name");
    let n1 = child_node(&t1, NodeId(1));
    let n2 = child_node(&t2, NodeId(1));
    let mut b1 = StringBuffer::acquire();
    let mut b2 = StringBuffer::acquire();
    let mut w1 = NodeWrapper {
        node: &n1,
        string_buffer: &mut b1,
    };
    let mut w2 = NodeWrapper {
        node: &n2,
        string_buffer: &mut b2,
    };
    let mut q2 = Vec::new();
    w2.enqueue_property_changes(&mut q2, &context, &element, &mut w1);
    assert!(property_change_ids(&q2).contains(&UIA_NamePropertyId));

    // Also verify descendant-derived names (Button labelled by child Label) are detected.
    let make_descendant_label_tree = |label_text: Option<&str>| {
        let mut root = Node::new(Role::Window);
        root.set_children(vec![NodeId(1)]);
        let mut button = Node::new(Role::Button);
        let mut nodes = vec![(NodeId(0), root)];
        if let Some(text) = label_text {
            button.set_children(vec![NodeId(2)]);
            let mut label = Node::new(Role::Label);
            label.set_value(text);
            nodes.push((NodeId(1), button));
            nodes.push((NodeId(2), label));
        } else {
            nodes.push((NodeId(1), button));
        }
        Tree::new(
            TreeUpdate {
                nodes,
                tree: Some(TreeInfo::new(NodeId(0))),
                tree_id: TreeId::ROOT,
                focus: NodeId(0),
            },
            true,
        )
    };
    let dt1 = make_descendant_label_tree(Some("Save"));
    let dt2 = make_descendant_label_tree(Some("Saved"));
    let dt3 = make_descendant_label_tree(None);
    let dn1 = child_node(&dt1, NodeId(1));
    let dn2 = child_node(&dt2, NodeId(1));
    let dn3 = child_node(&dt3, NodeId(1));
    let mut db1 = StringBuffer::acquire();
    let mut db2 = StringBuffer::acquire();
    let mut db3 = StringBuffer::acquire();
    let mut dw1 = NodeWrapper {
        node: &dn1,
        string_buffer: &mut db1,
    };
    let mut dw2 = NodeWrapper {
        node: &dn2,
        string_buffer: &mut db2,
    };
    let mut dw3 = NodeWrapper {
        node: &dn3,
        string_buffer: &mut db3,
    };
    let mut dq1 = Vec::new();
    dw2.enqueue_property_changes(&mut dq1, &context, &element, &mut dw1);
    assert!(property_change_ids(&dq1).contains(&UIA_NamePropertyId));

    let mut dq2 = Vec::new();
    dw3.enqueue_property_changes(&mut dq2, &context, &element, &mut dw2);
    assert!(property_change_ids(&dq2).contains(&UIA_NamePropertyId));
}
