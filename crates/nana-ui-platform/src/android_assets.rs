//! Android APK assets (Issue #274 §4).
//!
//! APK assets are not filesystem paths, so packaged resources are read
//! through the NDK `AAssetManager` of the application's `Context`. A large
//! asset stored uncompressed in the APK is handed out as the APK's file
//! descriptor plus its byte range ([`open_asset_range`]), so a resource pack
//! keeps its positional, per-entry reads; a small one (the package manifest)
//! is read whole ([`read_asset`]). Paths are asset-relative
//! (`nana/manifest/package.json`). Everywhere but Android both report
//! [`io::ErrorKind::Unsupported`].

use std::fs::File;
use std::io;

/// An uncompressed asset: `len` bytes of `file` (the APK) from `start`.
#[derive(Debug)]
pub struct AssetRange {
    pub file: File,
    pub start: u64,
    pub len: u64,
}

/// Open `path` for positional reads. A compressed asset cannot be read
/// this way and is refused ([`io::ErrorKind::InvalidData`]): the APK must
/// store it uncompressed (`androidResources.noCompress`).
pub fn open_asset_range(path: &str) -> io::Result<AssetRange> {
    imp::open_asset_range(path)
}

/// Read `path` whole. `Ok(None)` when the APK has no such asset; an asset
/// longer than `max_bytes` is refused before it is read.
pub fn read_asset(path: &str, max_bytes: u64) -> io::Result<Option<Vec<u8>>> {
    imp::read_asset(path, max_bytes)
}

#[cfg(not(target_os = "android"))]
mod imp {
    use super::AssetRange;
    use std::io;

    fn unsupported() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "APK assets exist only on Android",
        )
    }

    pub(super) fn open_asset_range(_: &str) -> io::Result<AssetRange> {
        Err(unsupported())
    }

    pub(super) fn read_asset(_: &str, _: u64) -> io::Result<Option<Vec<u8>>> {
        Err(unsupported())
    }
}

#[cfg(target_os = "android")]
mod imp {
    use super::AssetRange;
    use std::ffi::{CString, c_char, c_int, c_void};
    use std::fs::File;
    use std::io;
    use std::os::fd::FromRawFd;
    use std::sync::OnceLock;

    #[repr(C)]
    struct AAssetManager {
        _opaque: [u8; 0],
    }

    #[repr(C)]
    struct AAsset {
        _opaque: [u8; 0],
    }

    const AASSET_MODE_RANDOM: c_int = 1;
    const AASSET_MODE_STREAMING: c_int = 2;

    #[link(name = "android")]
    unsafe extern "C" {
        fn AAssetManager_fromJava(
            env: *mut jni::sys::JNIEnv,
            asset_manager: jni::sys::jobject,
        ) -> *mut AAssetManager;
        fn AAssetManager_open(
            manager: *mut AAssetManager,
            filename: *const c_char,
            mode: c_int,
        ) -> *mut AAsset;
        fn AAsset_close(asset: *mut AAsset);
        fn AAsset_read(asset: *mut AAsset, buf: *mut c_void, count: usize) -> c_int;
        fn AAsset_getLength64(asset: *mut AAsset) -> i64;
        fn AAsset_openFileDescriptor64(
            asset: *mut AAsset,
            out_start: *mut i64,
            out_length: *mut i64,
        ) -> c_int;
    }

    /// The native manager of the application's `AssetManager`. It is valid
    /// while the Java object lives, so a global reference to that object is
    /// held for the life of the process.
    struct Manager(*mut AAssetManager);

    // SAFETY: `AAssetManager` is documented as thread-safe; the pointer is
    // never freed (its Java object is pinned by a leaked global reference).
    unsafe impl Send for Manager {}
    unsafe impl Sync for Manager {}

    static MANAGER: OnceLock<Option<Manager>> = OnceLock::new();

    fn manager() -> io::Result<*mut AAssetManager> {
        MANAGER
            .get_or_init(|| {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(acquire))
                    .ok()
                    .flatten()
            })
            .as_ref()
            .map(|manager| manager.0)
            .ok_or_else(|| io::Error::other("the Android AssetManager is unavailable"))
    }

    fn acquire() -> Option<Manager> {
        use jni::JavaVM;
        use jni::objects::JObject;
        use std::mem::ManuallyDrop;

        let context = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(context.vm().cast()) }.ok()?;
        let mut env = vm.attach_current_thread().ok()?;
        let activity =
            ManuallyDrop::new(unsafe { JObject::from_raw(context.context() as jni::sys::jobject) });
        let assets = env
            .call_method(
                &*activity,
                "getAssets",
                "()Landroid/content/res/AssetManager;",
                &[],
            )
            .ok()?
            .l()
            .ok()?;
        if assets.is_null() {
            return None;
        }
        let global = env.new_global_ref(&assets).ok()?;
        let raw = unsafe { AAssetManager_fromJava(env.get_raw(), global.as_obj().as_raw()) };
        // Pinned for the process: the native manager borrows the Java object.
        std::mem::forget(global);
        (!raw.is_null()).then_some(Manager(raw))
    }

    /// An open asset, closed on drop.
    struct Asset(*mut AAsset);

    impl Drop for Asset {
        fn drop(&mut self) {
            unsafe { AAsset_close(self.0) };
        }
    }

    fn open(path: &str, mode: c_int) -> io::Result<Option<Asset>> {
        let name = CString::new(path)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "asset path contains NUL"))?;
        let asset = unsafe { AAssetManager_open(manager()?, name.as_ptr(), mode) };
        Ok((!asset.is_null()).then_some(Asset(asset)))
    }

    pub(super) fn open_asset_range(path: &str) -> io::Result<AssetRange> {
        let asset = open(path, AASSET_MODE_RANDOM)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no asset `{path}`")))?;
        let (mut start, mut len) = (0i64, 0i64);
        // The descriptor is a duplicate this process owns.
        let fd = unsafe { AAsset_openFileDescriptor64(asset.0, &mut start, &mut len) };
        if fd < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "asset `{path}` is compressed in the APK; store it uncompressed \
                     (androidResources.noCompress)"
                ),
            ));
        }
        let file = unsafe { File::from_raw_fd(fd) };
        let (Ok(start), Ok(len)) = (u64::try_from(start), u64::try_from(len)) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "negative asset range",
            ));
        };
        Ok(AssetRange { file, start, len })
    }

    pub(super) fn read_asset(path: &str, max_bytes: u64) -> io::Result<Option<Vec<u8>>> {
        let Some(asset) = open(path, AASSET_MODE_STREAMING)? else {
            return Ok(None);
        };
        let len = u64::try_from(unsafe { AAsset_getLength64(asset.0) })
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "negative asset length"))?;
        if len > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("asset `{path}` is larger than {max_bytes} bytes"),
            ));
        }
        let mut bytes = vec![0u8; len as usize];
        let mut done = 0usize;
        while done < bytes.len() {
            let read = unsafe {
                AAsset_read(
                    asset.0,
                    bytes[done..].as_mut_ptr().cast(),
                    bytes.len() - done,
                )
            };
            match read {
                n if n > 0 => done += n as usize,
                0 => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
                _ => return Err(io::Error::other(format!("cannot read asset `{path}`"))),
            }
        }
        Ok(Some(bytes))
    }
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    #[test]
    fn assets_are_android_only() {
        assert_eq!(
            super::open_asset_range("nana/manifest/package.json")
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::Unsupported
        );
        assert_eq!(
            super::read_asset("x", 1).unwrap_err().kind(),
            std::io::ErrorKind::Unsupported
        );
    }
}
