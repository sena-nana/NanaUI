//! URL 图片缓存的内存预算回归测试，覆盖 Base64 源与缓存键。

use super::*;

/// 构造 CPU/GPU 像素很小、编码源很大的离屏缓存条目。
#[test]
fn inactive_data_url_source_and_key_share_the_budget() {
    let mut cache = UrlTextureCache::default();
    cache.begin_frame();
    let url = format!("data:image/png;base64,{}", "A".repeat(34 * 1024 * 1024));
    let partition = cache.partition(&url);
    let mut entry = Entry::new(None, (1, 1), cache.frame);
    entry.source = Some(Source::Url(url.clone()));
    cache.bucket_mut(partition).entries.insert(url, entry);
    cache.begin_frame();
    cache.trim();
    assert!(
        cache.buckets.is_empty(),
        "编码源和缓存键合计超过 64 MiB 时释放离屏条目"
    );
}

/// 解码失败的 data URL 仍须受内存预算约束。
#[test]
fn failed_data_url_keys_are_bounded() {
    let mut cache = UrlTextureCache::default();
    cache.begin_frame();
    let url = format!(
        "data:image/png;base64,{}",
        "A".repeat(RETAINED_BYTES as usize)
    );
    cache.insert(url, None);
    cache.begin_frame();
    cache.trim();
    assert!(cache.buckets.is_empty(), "失败条目的缓存键也计入内存预算");
}

/// String 预留的容量会实际占用内存，因此也纳入驻留成本。
#[test]
fn url_source_accounts_for_reserved_capacity() {
    let mut url = String::with_capacity(1024);
    url.push_str("data:");
    let capacity = url.capacity() as u64;
    assert_eq!(Source::Url(url).retained_bytes(), capacity);
}
