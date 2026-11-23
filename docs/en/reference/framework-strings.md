# Framework Strings Table (`FrameworkStrings`)

Built-in framework controls ship with a set of default UI and accessibility strings: for example, the accessible names of the title bar's window buttons, the "Confirm / Cancel" of the confirm dialog, the media control bar's "Play / Pause / Fullscreen", the find-and-replace bar's placeholders, and the labels of the built-in appearance settings page. These strings are defined in one table, `nana_ui_core::FrameworkStrings` (Chinese by default). An application can replace the whole table or override any individual entries by key.

Strings an application passes explicitly to a specific control always take precedence (for example, `ConfirmDialog::confirm_label("Delete")` is not overridden by the table's default). `FrameworkStrings` is only the fallback source when the application has not specified a string.

## Replacing strings

### Rust path

```rust
let mut strings = nana_ui::runtime::FrameworkStrings::default();
strings.set("window.close", "Close");
strings.set("dialog.confirm", "OK");
context.set_framework_strings(strings)?;
```

Besides calling `.set(key, value)` by key name, you can read and write the struct fields directly (for example `strings.window_close = "Close".into()`). Set the table before mounting views. If you switch languages at run time and call `set_framework_strings`, mounted built-in compound controls are reassembled and reprojected with the new strings automatically.

## Parameterized template keys and key prefixes

Some entries contain brace placeholders (for example, `diff.reject_hunk` defaults to `"拒绝块{hunk}"`, `calendar.month` defaults to `"{month}月"`, and `toast.percent` defaults to `"{percent}%"`). When supplying translations, keep the brace parameters with the same names (for example `"Reject hunk {hunk}"`).

The full list of keys is available by enumerating `FrameworkStrings::KEYS`. Grouped by prefix:

| Key prefix | Built-in controls and surfaces |
| --- | --- |
| `window.*` | Names of the minimize, maximize, restore, and close buttons in the custom-drawn title bar |
| `dialog.*` | Default confirm and cancel button text of the confirm dialog (`ConfirmDialog`) |
| `toast.*` | The toast (`Toast`) close button (`toast.close`), progress percentage (`toast.percent`), and the notification stack's region name (`toast.region`) |
| `image_viewer.*` | Names of the image viewer (`ImageViewer`) close, previous, and next buttons |
| `media.*` | Names of the media transport bar (`MediaTransportBar`) play, pause, volume, fullscreen, progress, and settings buttons |
| `find.*` | Title, input placeholders, and action button text of the find-and-replace bar (`FindReplaceBar`) |
| `diff.*` | Action button text of the diff view |
| `calendar.*`, `date_picker.*` | Month/weekday formats of the calendar heatmap (`CalendarHeatmap`) and the date picker (`DatePicker`) |
| `command_palette.*`, `menu.search` | Search box placeholders of the command palette (`CommandPalette`) and searchable menus |
| `diagram.*`, `markdown.alert.*` | Accessible name and summary of a diagram in `NativeMarkdown`, and the titles of GFM alerts (`> [!NOTE]` and its four kin) |
| `theme.*`, `material.*`, `settings.*` | Theme, material, and setting labels of the built-in appearance settings page |
| Other standalone keys | `chip.remove`, `range_field.reset`, `file_tab.close`, `color_field.label`, `path_field.*`, `browser_view.label`, `dynamic_form.auto` |

In CI, `scripts/audit-framework-strings.py` scans the framework sources: any new Chinese UI literal added to built-in control code must go into the `FrameworkStrings` table and must not be scattered through component implementations.

## How switches are announced

The switch component (`Switch`) no longer writes a hard-coded `role_description` by default. The operating system's platform accessibility layer announces "开关" or "switch" according to the user's current system language. Set `AccessibilityState::role_description` explicitly only when an application wants a custom control to be announced as a specific type outside the platform's standard roles.
