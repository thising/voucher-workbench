# 第三方材料

本工具原创代码采用 [MIT](LICENSE)，不改变第三方代码或模型自身的许可。

- PP-OCRv5_mobile_det / PP-OCRv5_mobile_rec：PaddlePaddle 官方文字检测/识别模型，Apache-2.0。[固定版本与 hash](models/README.md)。权重由开发者从官方仓库下载，不纳入源码仓库。旧整页方向分类模型不再使用。
- ONNX Runtime：Microsoft，MIT。[源码与许可](https://github.com/microsoft/onnxruntime)。应用通过 `ort` 动态加载本地原生库，不调用 Python。
- PDF.js：Mozilla，Apache-2.0；离线预览固定为pdfjs-dist 6.5.438，来源及全部分发文件SHA256见 `web/pdfjs/manifest.json`。保留 `web/pdfjs/LICENSE`，CMap、标准字体及WASM解码组件各自完整许可随发行包保留；不从CDN加载。[上游](https://github.com/mozilla/pdf.js)。
- Tauri：MIT / Apache-2.0；桌面壳及其依赖以 `desktop/Cargo.lock` 和发行包中的完整Rust依赖许可为准。[上游](https://github.com/tauri-apps/tauri)。Windows WebView2为微软运行时，按微软分发条款独立提供，不改其许可；是否需要安装及离线附件见发行说明。
- `ort`：MIT / Apache-2.0；`image`：MIT / Apache-2.0；`calamine`、`lopdf`、`axum`、`tokio`：各自遵循上游许可。完整直接/间接依赖与版本以两个 `Cargo.lock` 为准，可用 `cargo metadata --locked` 查看各包的 license 字段与来源。

发行包连同实际分发的模型、预览组件和原生库保留上游完整 LICENSE/NOTICE。具体已构建/已验证的版本见 `../docs/windows-release.md`。开源及发行范围仅工具，不包含任何业务数据发布授权。

桌面包的 `licenses/rust-desktop-dependencies.json` 列出实际非开发依赖的固定版本、许可和对应 crates.io 源码归档地址，包括 MPL-2.0 的 selectors；补充许可文件的精确上游来源与校验值一并保留。
