# NanaUI vendor note

This directory contains `accesskit_winit` 0.33.2 from the AccessKit project.
NanaUI vendors the adapter so its `winit` dependency can be pinned to the same
Iced-owned revision used by the hosted runtime. Cargo patches from NanaUI's
workspace root are not inherited by downstream Git consumers, which otherwise
produces incompatible `winit` window and event-loop types in the accessibility
adapter.

The upstream crate is licensed under Apache-2.0 as declared in `Cargo.toml`.
