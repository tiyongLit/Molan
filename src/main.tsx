import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import './index.css'
// 全站统一滚动条契约（mole-scroll）：Clean/Optimize/Uninstall/Analyze 的 SimpleBar 共用
import './styles/scrollbar.css'
import { syncLocaleFromStore } from './i18n'
import { installSimpleBarDragGuard } from './utils/simpleBarDragGuard'

// 启动时从 settings store 水合界面语言（各窗口共用）；i18n 模块已同步读过 localStorage，
// 这里异步对齐持久化事实源，命中后 setLocale 会通知已挂载组件重渲染，不阻塞首屏。
void syncLocaleFromStore()

// SimpleBar 拖拽失联看门狗：拖拽中 mouseup 丢失（拖拽中切应用等）会让
// simplebar-dragging / preventClick 滞留并造成全应用"假死"，此守卫在指针
// 移动/滚轮时巡检并借库自身逻辑复位（背景与机制见模块头注释）。
installSimpleBarDragGuard()

// 生产构建屏蔽 WKWebView 原生右键菜单（Back/Forward/Reload 等导航项，Tauri 无配置项可关）；
// 可编辑元素保留系统编辑菜单（复制/粘贴），dev 环境保留原生菜单便于调试。
// 只 preventDefault 不拦截传播，Analyze 页的自定义右键菜单监听不受影响。
if (import.meta.env.PROD) {
  document.addEventListener('contextmenu', (e) => {
    const target = e.target as HTMLElement | null
    const editable =
      target instanceof HTMLInputElement ||
      target instanceof HTMLTextAreaElement ||
      (target?.isContentEditable ?? false)
    if (!editable) e.preventDefault()
  })
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
