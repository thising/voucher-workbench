# 第三方材料

本工具原创代码采用 [MIT](LICENSE)，不改变第三方代码或模型自身的许可。

- PP-OCRv5_mobile_det / PP-OCRv5_mobile_rec：PaddlePaddle 官方文字检测/识别模型，Apache-2.0。[固定版本与 hash](models/README.md)。权重由开发者从官方仓库下载，不纳入源码仓库。旧整页方向分类模型不再使用。
- ONNX Runtime：Microsoft，MIT。[源码与许可](https://github.com/microsoft/onnxruntime)。应用通过 `ort` 动态加载本地原生库，不调用 Python。
- `ort`：MIT / Apache-2.0；`image`：MIT / Apache-2.0；`calamine`、`lopdf`、`axum`、`tokio`：各自遵循上游许可。完整直接/间接依赖与版本以 `Cargo.lock` 为准，可用 `cargo metadata --locked` 查看各包的 license 字段与来源。

后续制作发行包时需连同实际分发的模型和原生库保留上游完整 LICENSE/NOTICE。本阶段尚未制作或发布安装包，不包含任何业务数据发布授权。
