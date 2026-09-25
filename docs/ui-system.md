# 默认插件 UI 规范

宿主和自带插件统一使用 `src-tauri/resources/ui-theme.css` 的语义变量。插件页面通过 `/_ui/theme.css` 读取由宿主提供的只读样式，不依赖彼此的安装目录，也不允许读取其他插件文件。宿主在返回样式时附加当前的浅色/深色选择和强调色；重新打开插件即可读取最新设置。

基础字色、背景、表面、边框、强调色分别使用 `--z-ui-text`、`--z-ui-background`、`--z-ui-surface`、`--z-ui-border`、`--z-ui-accent`。次级文字使用 `--z-ui-muted`，交互态使用 `--z-ui-surface-hover` 和 `--z-ui-accent-hover`。间距以 4px 倍数为主，普通圆角为 7–10px。

新页面使用以下公共组件类，不再为每个插件重写同类控件：

| 用途 | 类名 |
| --- | --- |
| 页面与标题区 | `.z-ui-page`、`.z-ui-page-header`、`.z-ui-title`、`.z-ui-description` |
| 内容与状态 | `.z-ui-card`、`.z-ui-field`、`.z-ui-empty` |
| 表单与操作 | `.z-ui-input`、`.z-ui-actions`、`.z-ui-button`、`.z-ui-button-primary` |

这些类仅规定视觉和基本布局。插件仍可通过自己的类调整内容密度和特殊交互，但不得重复定义基础色、焦点环与按钮高度。

截图标注保留原版工具布局，兼容层把强调色和表面色映射到上述变量；选区浮层和贴图保持深色以确保图片边界可辨。截图历史、剪贴板、休息提醒使用同一字色和强调色；截图历史与休息提醒直接使用公共组件类。第三方插件与百度、有道官网页面由其作者控制，宿主不强行覆盖其内部样式。
