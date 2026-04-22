/**
 * 与 `index.html` 内联脚本使用同一 key。
 * 用于在 JS bundle 加载前让 html/body 底色与当前 UI 明暗一致，减轻子窗口首帧闪白。
 */
export const CHROME_UI_STORAGE_KEY = 'flowshield.chromeUi' as const
