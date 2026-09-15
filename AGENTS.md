# AGENTS.md

## 注释规范

- 新增或实质修改的方法、函数，必须在声明正上方使用多行 JSDoc，简要说明职责。
- JSDoc 必须为每个参数提供 `@param`，并提供 `@returns`；方法可能主动抛错时补充 `@throws`。
- `void`、`Promise<void>` 方法也需要写明 `@returns`。
- 方法内部应在准备、校验、状态切换、资源发布、回滚、清理等关键步骤前添加单行注释。
- 修改实现时同步更新相关注释。

## 验证

- 前端必须通过 `pnpm typecheck` 和 `pnpm build:web`。
- Rust 必须通过 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings` 和测试。
- 桌面交互修改后必须以 `pnpm dev` 启动真实 Tauri 窗口验证，不使用打包产物代替开发态验证。
- 自动化测试必须使用临时应用数据目录，不能读写用户的正式数据。
