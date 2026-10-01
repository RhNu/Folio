# 架构总览

Folio 以项目为公开构建单位。项目服务装载清单、源码和依赖；语言前端与共享分析层产生带来源的语义事实；目标层把已检查的程序降为 MIR，经 PEX 后端和独立 codec 生成产物。CLI 与 LSP 是这些能力的适配层。

```text
folio.toml / 本地来源 / 编辑器缓冲区
  → 项目解析与一致输入
  → Papyrus 无损 CST
  → 声明、绑定、类型与 HIR
  → 目标合法化与 MIR 验证
  → PEX 后端 → PEX codec
  → 构建缓存与工作区输出
```

`fmt` 使用 CST；`lint` 与 IDE 查询使用同一分析视图。`check` 止于语义和目标可行性检查；`build` 继续编码和发布。文件与函数可以作为内部计算粒度，但公开构建始终具有项目上下文。

## 模块边界

| 区域 | 责任 |
| --- | --- |
| `foundation` | 文件身份、源码位置、结构化诊断和目标档案 |
| `project/model`、`project/resolve` | 清单领域模型、本地来源装载、依赖与脚本选择 |
| `languages/papyrus` | 词法、无损 CST、AST 和声明提取 |
| `compiler/hir`、`compiler/analysis` | 语义事实、名称与类型分析、一致查询视图 |
| `compiler/lowering`、`compiler/mir` | 目标降级、控制流与低层合法性 |
| `backends/pex`、`formats/pex` | MIR 到 PEX model；独立的 PEX 读写与校验 |
| `formats/declarations`、`tooling/declarations` | JSON/二进制声明载体及从 PSC、PEX 提取 API |
| `tooling/format`、`tooling/lint`、`tooling/ide` | 格式化、建议和编辑器查询 |
| `project/build` | 输入投影、构建计划、缓存、执行与产物提交 |
| `apps/cli`、`apps/lsp` | 命令、呈现、协议与进程边界 |
| `apps/xtask` | 仓库开发维护命令，不参与产品构建流程 |

crate 位于 `modules/<领域>/<短目录名>`，package 使用 `folio-<职责>`。分类目录不构成嵌套 workspace。依赖方向沿上表的领域能力流向应用：语法核心不依赖 Salsa、LSP 或清单；分析层不读磁盘；后端不读取 CST、不重新解析名称或类型；PEX codec 不依赖编译器。确有新的独立责任时才增加 crate。

## 输入、来源与诊断

项目层把磁盘内容、依赖选择、声明 API、目标和配置转换为显式分析输入。分析可保留局部错误并继续提供 IDE 事实；产物生成前严格拒绝未解决的错误和目标不合法的操作。文件身份和半开字节范围贯穿 CST、HIR、MIR 与诊断，生成节点保留原始来源。LSP 位置编码转换只发生在协议边界。

CLI 和 LSP 共用项目解析、语义规则、诊断和修复数据。打开的编辑器缓冲区覆盖磁盘文本；每批更新形成一致视图，过期结果不能发布为最新诊断。源码文本、声明 API 和目标中影响语义的内容进入查询或构建身份；缓存只是可丢弃的派生状态。

声明 API/API 的可见性与目标运行时能力分别建模。目标特性在具体使用点直接实现、等价降级或明确拒绝；Folio 不以静默近似改变程序语义。

## 工程约定

根 workspace 统一管理 Rust 工具链、依赖和锁文件。库通过 `tracing` 发出结构化事件，进程入口初始化 subscriber。项目加载、依赖选择、分析、降级、缓存决定与发布记录足够定位问题的上下文；源码正文和敏感数据不进入常规日志。CLI 日志写 stderr，LSP stdout 只承载协议。

具体契约分别见[项目模型](project-model.md)、[Papyrus 规范](../papyrus.md)、[编译管线](compiler.md)、[构建与产物](build-artifacts.md)及[工具与编辑器](tooling.md)。
