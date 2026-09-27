# Folio VS Code 客户端

此扩展为 `.psc` 提供基础语法高亮，将项目语义查询交给 `folio lsp`，显示诊断、带来源的 hover、定义与声明跳转、签名提示、文档大纲和语义高亮，并提供文档格式化。基础高亮使用通用 TextMate scope；服务器用 VS Code 标准语义 token 类别细化函数、事件、类型、属性、参数和变量的颜色。客户端源码位于本目录，可从仓库调试或打包成 VSIX 安装。

## 本地打包与安装

使用 Node.js 22 或更新版本，在本目录运行：

```powershell
npm ci
npm run package
code --install-extension .\folio-vscode-0.1.0.vsix
```

`npm run package` 会先编译 TypeScript，再生成 `folio-vscode-<版本>.vsix`。也可以在 VS Code 的扩展视图选择“从 VSIX 安装”。重复测试时重新打包并安装新 VSIX；版本号来自 `package.json`。VSIX 只包含客户端，安装后仍须将 `folio` 可执行文件加入 `PATH`，或设置 `folio.server.path`。

## 发布命令

`npm run publish` 会先编译客户端，然后调用 `vsce publish` 上传到 VS Code Marketplace。当前 `package.json` 中的 `folio-local` 仅用于本地测试；发布前须将它替换为已注册的 publisher ID、更新版本，并按[官方发布说明](https://code.visualstudio.com/api/working-with-extensions/publishing-extension)配置认证。本仓库不保存发布凭据。

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
| `folio.server.path` | Folio 可执行文件或其目录的绝对路径；未找到时继续搜索 `PATH` |
| `folio.server.manifestPath` | 可选清单路径；空值使用文件夹根目录的 `folio.toml` |
| `folio.server.logFilter` | 服务器日志过滤器，默认 `info` |

设置变化会重启服务器。扩展按 `folio.server.path`、`PATH` 的顺序寻找 `folio`；仅在 VS Code 扩展开发模式下，才回退到仓库的 `target/debug/folio`（Windows 为 `folio.exe`）。已配置路径不可用时，输出面板会记录实际采用的来源。LSP 报文走 stdout，日志走 stderr。仓库外的项目可使用 `PATH` 中的 Folio，或设置 `folio.server.path`。

在项目源码、PSC 目录和 package 依赖中，声明跳转指向解析后选中的 `.psc`。SDK、PEX 与内置声明没有可确认的本地 PSC 定义时，hover 显示提供者信息，跳转不返回位置。主题可直接使用通用 TextMate scope 与 VS Code 的标准语义 token 颜色规则。
