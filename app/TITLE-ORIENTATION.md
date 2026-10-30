# 标题文字方向判定接口与实施依据

2026-10-30 用户明确要求替换整页分类，只依据标题文字方向判定旋转。图片尺寸只用于旋转后PDF横竖排版。不得混入旧分类结果、按文件夹猜方向或把同组所有页面强制相同方向。

当前要求：方向识别默认不介入；启用必须同意免责声明，并在识别完成后人工核对全部待输出图片。当前工作流见[有效规格](../openspec/specs/voucher-pdf/spec.md)。以下算法未决统计不等同于工作流人工待核数量。原生运行库初始化时显式关闭telemetry。

## 模块契约
- `ocr.rs`: `pub struct OcrEngine`; `new(model_dir:&Path,runtime:Option<&Path>)->Result<Self>`; `read_lines(&mut self,image:&image::RgbImage)->Result<Vec<TextLine>>`。TextLine `{text:String,confidence:f32,bbox:[u32;4]}`，bbox为输入图的 x/y/width/height。文字检测+文字识别，无文档/文字方向分类和自动旋转。原生ort动态加载继续使用，不增加Python运行依赖。
- `orientation.rs`: 对外保持 `Engine::new(model_path:&Path,runtime:Option<&Path>)` 与 `predict(path:&Path)->Result<Orientation>`，`model_path`改指OCR模型目录；保留EXIF、白底和原生runtime辅助函数供OCR复用。新算法`MODEL_ID`前缀 `title-ocr-v1`。
- 图像按顺时针0/90/180/270生成候选，只在各候选顶部42%区域检测/识别；返回标题文本、OCR置信度、候选框、关键词、接受与否。无标题/模糊/冲突需复核，不能通过降低旧分类0.9阈值放行。
- 当前标题条件：通用凭证/发票/委托书等标题词尾，排除发票字段、买卖方记账凭证侧注和联次说明；OCR字符平均分至少0.82、框高至少7px、宽至少1.5倍高、横向中心位于6%至94%。这些是可读性门槛，不是方向正确率。多方向均识别到标题时保留复核。
- Orientation的`needs_review`仅表示模型未决，`title`、`reason`、`candidates`为建议证据，confidence仅为OCR字符分数。启用识别时输出门禁由workflow和人工reviewed决定：所有图片均须人工确认。旧scores只用于兼容，不参与决策。
- 非人工确认且结果model不是当前title算法的图片，在启用识别时须重新分析；禁用时保留证据但不应用自动角度。

## 验收
- 用既有整批3270图独立运行，统计自动通过/待复核与方向分布，和旧2052/3270比较。
- 五页已知样例0/0/0/0/270；增加它们的90/180/270合成旋转变体，检验非零方向纠正。
- 实际查看全部建议旋转页面；查看未决页面；对自动0度按凭证/置信度/不同标题分层抽样。将“检查范围内正确率”和“全批复核率”分开报告，不能以模型自评替代ground truth。
- 不发布业务数据，不制作安装包；旧批次和状态保留可回滚。
