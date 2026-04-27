import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import './index.css'
// 全站统一滚动条契约（mole-scroll）：Clean/Optimize/Uninstall/Analyze 的 SimpleBar 共用
import './styles/scrollbar.css'

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
