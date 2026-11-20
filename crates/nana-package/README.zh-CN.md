# nana-package

[English](README.md) · 简体中文

Nana 打包格式的读取侧：包清单、`.nrpack` 资源包、内容密钥和发布者签名，随应用一起发布。

`PackReader` 随机访问 `.nrpack`，遇到任何不一致就拒绝：先校验 Header、发布者签名和 TOC 哈希，再按需逐块校验、解密、解压。内容密钥由应用实现的 `KeyProvider` 提供，是否要求发布者签名由 `TrustPolicy` 决定。资源包可以从文件读，也可以从文件中的一段字节读（`FileRange`，Android APK 中不压缩的资源就这样读）。`update` 模块定义已签名的更新清单与回滚日志，供应用自己的更新器使用。因为会链接进发行的应用，它只依赖纯 Rust 库，`scripts/check-package-boundary.py` 会检查这一点。

应用通常不直接用它，而是通过 `nana-ui` 的 `packaged-resources` feature 和 `ResourcePackOptions` 挂载资源包。构建资源包、生成密钥和签名在构建期的 [`nana-packager`](../nana-packager) 里。

## 测试

```bash
cargo test -p nana-package
python3 scripts/check-package-boundary.py
```

更多内容见 [打包与分发](../../docs/reference/packaging.md) 和 [打包指南](../../docs/guide/scaling/packaging.md)。
