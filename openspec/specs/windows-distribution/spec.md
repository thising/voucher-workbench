# Windows发行规格

## Purpose

提供可重复构建、仅含工具的Windows x64测试包，并区分CI运行与客户Windows10验收。

## Requirements

### Requirement: 可复现的Windows测试包
系统 SHALL 在Windows x64 MSVC环境从固定源码、Cargo.lock和固定校验的模型/原生库构建ZIP，包含运行所需工具材料与第三方许可，提供双击启动和完整hash清单。不得将业务资料、历史状态、日志或预览上传至源码仓库或CI。

#### Scenario: 干净构建
- **WHEN** Windows CI从仅工具的源码快照执行构建
- **THEN** 完成测试、release构建、依赖校验、随包运行库加载和合成资料冒烟后，才上传测试构件；不自动发布公开Release。

### Requirement: 与工作目录无关的启动
系统 SHALL 从可执行文件位置寻找随包模型和运行库，在Windows默认使用用户LOCALAPPDATA下的稳定状态目录，并保留显式CLI覆盖。依赖不可用时仍允许原向合并；重复启动和错误有可见提示。

#### Scenario: 中文路径与重启
- **WHEN** 用户解压至中文含空格目录并从不同工作目录启动、关闭再启动
- **THEN** 随包组件正常定位、历史保持，关闭识别与人工修正规则不变；未就绪的OCR不阻止原向生成。

### Requirement: 分层验收与授权
系统 SHALL 区分本地代码验证、Windows CI实际运行和客户Windows10实际验收。仓库选择、外部写入与公开分发遵守用户明确授权；不要求用户在聊天中提供密码或token。

#### Scenario: 外部条件未满足
- **WHEN** 仓库或Windows真机条件尚未提供
- **THEN** 本地完成可审阅的配置、脚本和仅工具源码包，明确剩余支持，不把编译检查或CI配置文件视为Windows成品。
