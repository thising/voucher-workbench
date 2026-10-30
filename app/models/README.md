# 本地标题 OCR 模型

当前采用 PaddlePaddle 官方 **PP-OCRv5_mobile_det + PP-OCRv5_mobile_rec** 的 ONNX 模型。它们只检测和识别文字，不分类整页方向、不自动旋转文字框。上层用0/90/180/270四个候选中的正常可读标题决定是否旋转。

模型在用户计算机CPU上推理；应用运行时不联网、不下载、不上传资料，不需要Python或Office。旧 `inference.onnx` 整页方向模型不再参与识别。

## 固定模型与来源

| 文件 | 官方版本 | SHA256 |
|---|---|---|
| `title-ocr/det.onnx`，4,826,518字节 | [PP-OCRv5_mobile_det_onnx@e6f4fa85](https://huggingface.co/PaddlePaddle/PP-OCRv5_mobile_det_onnx/tree/e6f4fa85f00e168c862bc462aebca69eef9b3d3d) | `a431985659dc921974177a95adcfbb90fd9e51989a5e04d70d0b75f597b6e61d` |
| `title-ocr/rec.onnx`，16,534,782字节 | [PP-OCRv5_mobile_rec_onnx@ed152b8b](https://huggingface.co/PaddlePaddle/PP-OCRv5_mobile_rec_onnx/tree/ed152b8b495f84de93cda5709d768548a9127622) | `da72dc72ca4dc220df0dfde68c1dedc31c58d3e76a25871122e5056227d50092` |
| `title-ocr/dictionary.txt`，18,383条 | 从同版本rec.yml的character_dict无损提取 | `d1979e9f794c464c0d2e0b70a7fe14dd978e9dc644c0e71f14158cdf8342af1b` |

下载：`sh app/tools/fetch-title-models.sh`。模型权重不进源码仓库，脚本和程序均核验SHA256。Windows开发可按 [manifest.json](title-ocr/manifest.json) 固定URL下载到对应位置，用 `Get-FileHash -Algorithm SHA256` 校验。

上游模型、字典和配置文件为Apache-2.0材料，作者PaddlePaddle；见 [LICENSE](title-ocr/LICENSE)。这些材料不属于本项目原创MIT代码许可。后续分发需保留上游许可和来源说明。

## 预处理与限制

官方两个配置的DecodeImage均为 **BGR**。Rust接口输入RGB，预处理时转换BGR；不能把默认RGB读取器误认为模型实际通道顺序。

- 检测：长边最多960，尺寸取32倍数；ImageNet均值/标准差；DB概率阈值0.3，组件均值阈值0.6。为避免自动转正掩盖候选方向，仅保留水平框，使用纯Rust八连通域与轴对齐DB扩框，识别12个较显著文字行；另允许最多4个字号、基线与距离接近的相邻双框联合识别，处理印章造成的标题断裂，不递归拼接整行正文。
- 识别：高48，宽随行宽高比变化，最少320、最多3200；归一化到[-1,1]，归一化后零填充；CTC解码合并相邻重复并移除blank。字典从官方配置提取，前加blank、后加space，共18,385类别。
- 上游预处理配置留存于 `title-ocr/det.yml` / `rec.yml`。文本框提取为本项目适配，非宣称逐像素复刻OpenCV多边形后处理；不适用于一般斜排文字排版恢复。
- OCR置信度表示所识别字符的模型分数，不是方向准确率。标题接受、冲突和待复核规则在应用方向模块中判定。

原生运行库为 [ONNX Runtime](https://github.com/microsoft/onnxruntime)，MIT许可。Rust `ort=2.0.0-rc.10` 动态加载本机DLL/dylib/so，开发实际验证版本1.23.2。启动 `--runtime` 或 `ORT_DYLIB_PATH` 指定对应架构运行库。
