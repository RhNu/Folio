# Folio

Folio 是面向 Papyrus 项目的工具链。它读取 `folio.toml`，解析本地源码和声明依赖，检查 Skyrim Papyrus，并为根项目生成 PEX。检查、构建、格式化、lint 和编辑器服务使用同一项目上下文。

## 快速开始

以下命令假定 `folio` 已在 `PATH` 中：

```powershell
folio new MyMod
cd MyMod
folio check
folio build
```

`folio new` 创建项目目录、`folio.toml` 和 `Source/Scripts` 中的起始脚本。已有项目可在项目目录运行 `folio init --name MyMod`。命令从当前目录向上查找最近的 `folio.toml`；也可传入 `--manifest-path <路径>` 指定清单。

默认从 `Source/Scripts` 读取 `.psc`，将根项目生成的 `.pex` 放在 `Scripts`。`.folio` 保存可丢弃的缓存和构建记录。Folio 不向游戏目录部署文件；本地依赖只供分析。

## 常用命令

| 命令                         | 用途                                   |
| ---------------------------- | -------------------------------------- |
| `folio check`                | 检查项目语义与目标可行性，不写 PEX     |
| `folio build`                | 构建并发布根项目的 PEX                 |
| `folio fmt`                  | 检查源码格式；`folio fmt --write` 写回 |
| `folio lint`                 | 报告可配置的代码建议                   |
| `folio tree`                 | 查看依赖和同名脚本的选择               |
| `folio metadata`             | 输出解析后的项目数据，默认 JSON        |
| `folio inspect`              | 验证最近一次成功构建及产物             |
| `folio inspect --pex <路径>` | 独立读取一个 PEX 文件                  |
| `folio declarations list`    | 列出内置声明包                         |
| `folio lsp`                  | 启动供编辑器使用的语言服务器           |

`check` 不执行最终 PEX 编码，因此个别布局限制可能在 `build` 时才报告。`inspect --pex` 只读取产物，不执行单文件构建。`check`、`build`、`lint`、`tree` 和 `inspect` 可按命令帮助选择文本或 JSON 输出。

## 项目配置

`folio.toml` 使用 schema 3。下面是基本配置；`paths` 可省略，展示的是默认目录：

```toml
schema = 3

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

需要 CK 或 SKSE API 时，可用 `folio declarations list` 查看内置包，并在清单中显式声明依赖。后列依赖的同名脚本优先；例如 CK 在前、SKSE 在后。声明仅提供分析时的 API 可见性，所需运行时仍由项目自行提供。完整字段、依赖示例和覆盖规则见[项目与构建](docs/architecture/project-model.md)。

只有 `.psc` 的模组源码目录可直接作为声明依赖，无需给该目录添加 `folio.toml`：

```toml
[[dependencies]]
name = "other-mod"
kind = "psc"
path = "../OtherMod/Source/Scripts"
```

Folio 当前接受的语言构造及 Skyrim 行为见[Papyrus 规范](docs/papyrus.md)。格式化、lint 和编辑器功能见[工具与编辑器](docs/architecture/tooling.md)。

## 开发指南

仓库按 `modules/<领域>/<crate>` 组织 Rust workspace；`editors/vscode` 是 VS Code 客户端。阅读[架构总览](docs/architecture/overview.md)了解模块边界，[编译管线](docs/architecture/compiler.md)了解语义、降级和生成，[构建与产物](docs/architecture/build-artifacts.md)了解缓存与发布。未完成工作集中在[开发计划](docs/planning/roadmap.md)。

修改公开配置或行为时，更新所属专题和本页相关用法。开发约束见[AGENTS.md](AGENTS.md)。VS Code 客户端的本地调试方式见[扩展说明](editors/vscode/README.md)。

仓库维护命令通过 `cargo xtask` 运行。`cargo xtask lines` 统计 `modules` 下各 Rust 源文件的非空、非纯注释行；超过 650 行提示警告，超过 1200 行报错并返回非零状态。行尾注释所在的代码行仍计入。超过硬限制时应按职责拆入子模块，较大的内嵌测试也可移入测试子模块。

## 许可与来源

Folio 使用 [GNU GPL version 3](LICENSE)。复用源码的来源见[PEX codec 来源记录](modules/formats/pex/PROVENANCE.md)；内置声明的输入与摘要见[声明来源记录](modules/formats/declarations/builtin/README.md)。第三方 SDK、游戏脚本和其他资产须按各自权利条件使用和分发。
