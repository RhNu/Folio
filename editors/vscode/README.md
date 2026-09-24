# Folio VS Code 客户端

此扩展将 `.psc` 文件交给 `folio lsp`，显示诊断、hover 和跳转定义，并提供文档格式化。语言与项目规则由 Folio 服务器处理。客户端源码位于本目录，目前从仓库运行。

## 本地调试

1. 在 `editors/vscode` 安装 JavaScript 依赖，然后用 VS Code 打开仓库。
2. 在“运行和调试”选择 **Folio: Debug VS Code extension**，按 F5。预启动任务构建扩展和 Folio，开发宿主打开 `fixtures/editor-project`。
3. 打开 `Source/Scripts/FolioArenaController.psc`。扩展启动服务器后，“Folio Language Server”面板显示连接和错误信息。
4. 调试服务器时，在仓库窗口运行 **Folio: Attach to LSP process**，选择 Folio 进程。此调试配置使用 CodeLLDB。

修改扩展或 Rust 代码后重新启动 F5 调试会话。命令面板中的 **Folio: Restart Language Server** 使用当前二进制重启服务器。

## 项目与设置

VS Code 应打开含 `folio.toml` 的项目文件夹。一个客户端会话处理一个项目文件夹内的 `.psc`，并转发该文件夹内的清单、源码和 JSON 文件变化。

| 设置 | 用途 |
| --- | --- |
| `folio.server.path` | Folio 可执行文件的绝对路径；空值使用仓库调试构建 |
| `folio.server.manifestPath` | 可选清单路径；空值使用文件夹根目录的 `folio.toml` |
| `folio.server.logFilter` | 服务器日志过滤器，默认 `info` |

设置变化会重启服务器。LSP 报文走 stdout，日志走 stderr。仓库外的项目可在开发宿主中打开其文件夹，并设置 `folio.server.path`。
