import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import './index.css'
// 全站统一滚动条契约（mole-scroll）：Clean/Optimize/Uninstall/Analyze 的 SimpleBar 共用
import './styles/scrollbar.css'
import { syncLocaleFromStore } from './i18n'

// 启动时从 settings store 水合界面语言（各窗口共用）；i18n 模块已同步读过 localStorage，
// 这里异步对齐持久化事实源，命中后 setLocale 会通知已挂载组件重渲染，不阻塞首屏。
void syncLocaleFromStore()

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
