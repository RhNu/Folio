# Folio

[English](README.md) | [简体中文](README.zh.md)

Folio 是面向 Papyrus 项目的工具链。它读取 `folio.toml`，加载本地源码和声明依赖，检查 Skyrim Papyrus，并为根项目生成 PEX。检查、构建、格式化、lint 和编辑器服务共享同一项目上下文。

## 快速开始

将 `folio` 加入 `PATH` 后，运行：

```powershell
folio new MyMod
cd MyMod
folio check
folio build
```

`folio new` 创建清单和 `Source/Scripts` 下的起始脚本。要初始化已有目录，在该目录运行 `folio init --name MyMod`。项目命令从当前目录向上查找最近的 `folio.toml`；也可用 `--manifest-path <path>` 显式指定清单。

默认情况下，Folio 从 `Source/Scripts` 读取 `.psc`，将根项目的 `.pex` 写入 `Scripts`，并在 `.folio` 中保存可丢弃的缓存与构建记录。依赖为分析提供声明。Folio 不向游戏目录部署文件。

## 常用命令

| 命令                         | 用途                                      |
| ---------------------------- | ----------------------------------------- |
| `folio check`                | 检查语义与目标可行性，不写 PEX            |
| `folio build`                | 为根项目生成并发布 PEX                    |
| `folio fmt`                  | 检查格式；用 `folio fmt --write` 应用修改 |
| `folio lint`                 | 报告可配置的代码建议                      |
| `folio tree`                 | 查看依赖与脚本提供者的选择                |
| `folio metadata`             | 描述解析后的项目，默认输出 JSON           |
| `folio inspect`              | 验证最近一次成功构建及其产物              |
| `folio inspect --pex <path>` | 独立于项目读取 PEX 文件                   |
| `folio declarations list`    | 列出本地仓库中的声明                      |
| `folio lsp`                  | 启动语言服务器                            |

`check` 不执行最终 PEX 编码，因此 `build` 可能报告额外的布局限制。`inspect --pex` 读取产物，不构建源文件。各命令的选项和输出格式见相应命令帮助。

## 项目配置

清单使用当前版本的字段与默认值，不声明 schema 版本。下例展示默认源码与输出目录；`[paths]` 可省略。

```toml
[package]
name = "my-mod"
version = "0.1.0"

[paths]
source = "Source/Scripts"
output = "Scripts"

[languages.papyrus]
dialect = "skyrim"
extensions = ["psc"]

[build]
target = "skyrim-se"
profile = "dev"
emit = ["pex"]
```

依赖支持 PSC 目录（`psc`）、实验性的 PEX 目录（`pex`）、声明文件（`decl`）和全局本地仓库条目（`repo`）。四种输入都提供 API 声明。后列依赖按整脚本覆盖前面的同名脚本；根项目源码优先于所有依赖。项目仍须自行提供所需的运行时实现。

模组的 PSC 目录可以直接使用，无需为其添加清单：

```toml
[[dependencies]]
name = "other-mod"
kind = "psc"
path = "../OtherMod/Source/Scripts"
```

`decl` 接受 JSON 和二进制 `.fdecl` 文件。`repo` 在用户目录的 `.folio/repo` 下查找声明；绝对路径的 `FOLIO_HOME` 可改变该位置。CK 与 SKSE 声明应从自己的本地源码生成，并在项目中显式选择。PEX 目录依赖要求开启 `[experimental] pex-dependencies = true`，且只提供二进制中可还原的 API 事实。配置、生成命令和限制见[项目与依赖](docs/architecture/project-model.md)。

## 语言与编辑器支持

[Papyrus 语言参考](docs/papyrus.md) 整理 Skyrim 基线、[游戏方言差异](docs/papyrus/dialects.md)和[引擎语义](docs/papyrus/runtime.md)，并注明来源及尚未解决的规范问题。Folio 实现 `skyrim` 方言和 `skyrim-se` 生成目标。源码与依赖声明允许带默认值的参数位于必填参数之前，并共享字面量默认值、属性形式和声明标志校验；语义分析检查词法作用域、转换、访问器权限和继承契约。[编译管线](docs/architecture/compiler.md) 说明当前实现。Folio 扩展和尚未完成的 CK／引擎验证仍明确记录在参考文档及[符合性路线图](docs/planning/roadmap.md#papyrus-conformance)中。[工具与编辑器服务](docs/architecture/tooling.md) 介绍格式化、lint、诊断、导航及语言服务器的项目模型。

VS Code 客户端提供 `.psc` 文件图标、语法与语义高亮、带文档和来源链接的声明 hover、针对关键字、内置类型、字面量和运算符的离线 Skyrim 语言 hover、补全、签名提示、引用查询、继承导航、CodeLens、参数提示、经过验证的项目符号重命名、符号搜索和格式化。语言 hover 在源码和只读 API 文档中提供字面量值、示例与 Creation Kit 参考链接。没有 PSC 源码的依赖以只读 API 声明视图打开。它优先使用配置的 Folio 可执行文件或目录，然后搜索 `PATH`。在 `editors/vscode` 运行 `npm ci` 和 `npm run package` 可打包客户端。VSIX 不包含 Folio 可执行文件。安装、设置、调试和发布方式见[扩展 README](editors/vscode/README.md)。

## 开发指南

Rust crate 位于 `modules/<domain>/<crate>`，VS Code 客户端位于 `editors/vscode`。先阅读[架构总览](docs/architecture/overview.md)，再查看相关专题：

- [项目与依赖](docs/architecture/project-model.md)：清单、声明和提供者选择。
- [编译管线](docs/architecture/compiler.md)：分析、降级和 PEX 生成。
- [构建与产物](docs/architecture/build-artifacts.md)：指纹、缓存和输出发布。
- [工具与编辑器服务](docs/architecture/tooling.md)：CLI、LSP、格式化、lint 和仓库维护。

[AGENTS.md](AGENTS.md) 记录项目约束。公开配置或行为变化时，同步更新两个语言版本的 README 和所属专题。未完成工作与验证缺口集中在[路线图](docs/planning/roadmap.md)。

运行 `cargo xtask check-lines` 可统计整个 workspace 的 Rust 代码行数。它排除注释、空白行和生成目录；单文件超过 650 行时警告，超过 1,200 行时失败。`--all` 列出全部文件，`--manifest-path <Cargo.toml>` 指定 workspace。计数和退出状态规则见[仓库维护](docs/architecture/tooling.md#repository-maintenance)。

## 许可与来源

Folio 使用 [GNU GPL version 3](LICENSE)。[声明生成](docs/architecture/project-model.md#generating-declarations) 说明本地 API 来源与归属信息。第三方 SDK、游戏脚本及其他资产仍须遵守各自的使用与分发条件。
