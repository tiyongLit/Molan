/**
 * 路由 chunk 预加载（首访卡顿治理）。
 *
 * 日志实证：src/routers.tsx 所有页面均为 React.lazy 动态导入；
 * 首次进入某页面时需加载 chunk（dev 下 vite 现场编译可达 2.4s），
 * 第二次访问 chunk 已缓存后降至 200-300ms。
 *
 * 本模块在启动空闲期后台顺序加载各页面 chunk，用户点击时
 * React.lazy 直接命中 ESM 缓存，消除 chunk 等待时间。
 *
 * 预加载的 import() 与 React.lazy 的 factory 共享模块缓存：
 * 预加载执行后，后续 React.lazy 的 import() 返回已缓存的 Promise。
 */

/** 预加载列表：按权重排序（最重/最常用优先） */
const PRELOADERS: Array<{ id: string; load: () => Promise<unknown> }> = [
  { id: 'clean',     load: () => import(/* @vite-ignore */ '@/pages/Clean')     },
  { id: 'uninstall', load: () => import(/* @vite-ignore */ '@/pages/Uninstall') },
  { id: 'optimize',  load: () => import(/* @vite-ignore */ '@/pages/Optimize')  },
  { id: 'analyze',   load: () => import(/* @vite-ignore */ '@/pages/Analyze')   },
]

/** 已预加载集合（去重：同一 id 只加载一次） */
const preloaded = new Set<string>()

/**
 * 预加载单个路由 chunk（侧边栏 hover 触发）。
 * 已加载过的立即返回（ESM 缓存命中）。
 */
export function preloadRoute(id: string): void {
  if (preloaded.has(id)) return
  preloaded.add(id)
  const entry = PRELOADERS.find((p) => p.id === id)
  if (entry) {
    entry.load().catch(() => {
      // 预加载失败不影响正常导航（React.lazy 会自己加载）
      preloaded.delete(id)
    })
  }
}

/**
 * 启动全量预加载：按顺序逐个加载（上一个完成再加载下一个）。
 * 避免并发请求风暴，让出主线程给首屏渲染。
 *
 * 调用方（layout/index.tsx）应在启动 3 秒后调度，
 * 确保 F0/F1 采集和首页渲染完成后再开始后台预加载。
 */
export function preloadAllRoutes(): void {
  PRELOADERS.reduce<Promise<unknown>>(
    (chain, { id, load }) =>
      chain.then(() => {
        if (preloaded.has(id)) return
        preloaded.add(id)
        return load().catch(() => {
          preloaded.delete(id)
        })
      }),
    Promise.resolve()
  )
}
