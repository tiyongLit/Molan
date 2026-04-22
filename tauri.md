```ts
// src/windowUtils.ts
import { invoke } from "@tauri-apps/api/core";

/**
 * 平滑调整 Tauri 窗口尺寸（带动画过渡）
 * @param targetWidth 目标宽度（逻辑像素）
 * @param targetHeight 目标高度（逻辑像素）
 * @param duration 动画持续时间（毫秒，默认 300）
 * @param steps 动画步数（默认 30）
 */
export async function smoothResizeWindow(
  targetWidth: number,
  targetHeight: number,
  duration: number = 300,
  steps: number = 30
): Promise<void> {
  // 获取当前窗口尺寸（逻辑像素）
  const { width: currentWidth, height: currentHeight } = await invoke<{
    width: number;
    height: number;
  }>("get_window_size");

  // 执行步进式 resize
  for (let i = 0; i <= steps; i++) {
    const t = i / steps;
    const w = currentWidth + (targetWidth - currentWidth) * t;
    const h = currentHeight + (targetHeight - currentHeight) * t;
    await invoke("set_window_size", { width: w, height: h });
    await new Promise((resolve) => setTimeout(resolve, duration / steps));
  }
}
```

```tsx
import { useState } from "react";
import reactLogo from "./assets/react.svg";
import { invoke } from "@tauri-apps/api/core";
import { smoothResizeWindow } from "./windowUtils"; // ← 引入工具函数

import "./App.css";

function App() {
  const [greetMsg, setGreetMsg] = useState("");
  const [name, setName] = useState("");

  async function greet() {
    // Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
    setGreetMsg(await invoke("greet", { name }));
  }

 async function autoResizeWindow() {
    await smoothResizeWindow(1000, 618);
  }

  async function autoResizeWindow2() {
    await smoothResizeWindow(318, 618);
  }

  return (
    <main className="container">
      <h1>Welcome to Tauri + React</h1>

      <div className="row">
        <a href="https://vite.dev" target="_blank">
          <img src="/vite.svg" className="logo vite" alt="Vite logo" />
        </a>
        <a href="https://tauri.app" target="_blank">
          <img src="/tauri.svg" className="logo tauri" alt="Tauri logo" />
        </a>
        <a href="https://react.dev" target="_blank">
          <img src={reactLogo} className="logo react" alt="React logo" />
        </a>
      </div>
      <p>Click on the Tauri, Vite, and React logos to learn more.</p>

      <form
        className="row"
        onSubmit={(e) => {
          e.preventDefault();
          autoResizeWindow();
        }}
      >
        <input
          id="greet-input"
          onChange={(e) => setName(e.currentTarget.value)}
          placeholder="Enter a name..."
        />
        <button type="submit">Greet</button>
      </form>
      <p>{greetMsg}</p>

      <form
        className="row"
        onSubmit={(e) => {
          e.preventDefault();
          autoResizeWindow2();
        }}
      >
        <button type="submit">G23reet</button>
      </form>
    </main>
  );
}

export default App;

```


```rs
use tauri::{Window};


// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![greet, set_window_size, get_window_size])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[tauri::command]
async fn set_window_size(window: Window, width: f64, height: f64) -> Result<(), String> {
    window
        .set_size(tauri::Size::Logical(tauri::LogicalSize { width, height }))
        .map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
struct WindowSize {
    width: f64,
    height: f64,
}

#[tauri::command]
async fn get_window_size(window: Window) -> Result<WindowSize, String> {
    let physical_size = window.inner_size()
        .map_err(|e| e.to_string())?;

    let scale_factor = window.scale_factor()
        .map_err(|e| e.to_string())?;

    let logical_width = physical_size.width as f64 / scale_factor;
    let logical_height = physical_size.height as f64 / scale_factor;

    Ok(WindowSize {
        width: logical_width,
        height: logical_height,
    })
}
```
