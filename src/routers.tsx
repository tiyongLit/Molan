import { lazy, Suspense } from 'react'
import MainLayout from './layout'
import { createBrowserRouter, Navigate } from 'react-router-dom'

// ── 路由级代码分割：每个页面独立 chunk，首次切到该路由时才加载 ──
const Home = lazy(() => import('@/pages/Home').then(m => ({ default: m.Home })))
const Clean = lazy(() => import('@/pages/Clean').then(m => ({ default: m.Clean })))
const ShellUninstall = lazy(() => import('@/pages/Uninstall').then(m => ({ default: m.ShellUninstall })))
const Optimize = lazy(() => import('@/pages/Optimize').then(m => ({ default: m.Optimize })))
const Analyze = lazy(() => import('@/pages/Analyze').then(m => ({ default: m.Analyze })))
const Dashboard = lazy(() => import('@/pages/Dashboard').then(m => ({ default: m.Dashboard })))
const Settings = lazy(() => import('@/pages/Settings').then(m => ({ default: m.Settings })))
const TrashReminderWindow = lazy(() => import('@/pages/TrashReminderWindow').then(m => ({ default: m.TrashReminderWindow })))
const FdaGuideWindow = lazy(() => import('@/pages/FdaGuideWindow').then(m => ({ default: m.FdaGuideWindow })))

function PageFallback() {
  return <div className="h-full w-full" />
}

export const router = createBrowserRouter([
    {
        // 系统托盘仪表盘气泡：独立窗口（tauri.conf.json label=dashboard），
        // 不套 MainLayout（无侧边栏），顶层路由。
        path: '/dashboard',
        element: (
          <Suspense fallback={<PageFallback />}>
            <Dashboard />
          </Suspense>
        ),
    },
    {
        // 设置窗口：独立窗口（tauri.conf.json label=settings），
        // 不套 MainLayout，顶层路由。
        path: '/settings',
        element: (
          <Suspense fallback={<PageFallback />}>
            <Settings />
          </Suspense>
        ),
    },
    {
        // 废纸篓超阈值提醒浮窗：独立窗口（tauri.conf.json label=trash-reminder），
        // 右上角弹出，不套 MainLayout，顶层路由。
        path: '/trash-reminder',
        element: (
          <Suspense fallback={<PageFallback />}>
            <TrashReminderWindow />
          </Suspense>
        ),
    },
    {
        // FDA 权限引导窗：独立窗口（Rust 懒创建 label=fda-guide），
        // 权限被拒时由 fda_guide 弹出引导授权，不套 MainLayout，顶层路由。
        path: '/fda-guide',
        element: (
          <Suspense fallback={<PageFallback />}>
            <FdaGuideWindow />
          </Suspense>
        ),
    },
    {
        path: '/',
        element: <MainLayout />,
        children: [
            {
                index: true,
                element: <Navigate to="/home" replace />,
            },
            {
                path: 'home',
                element: (
                  <Suspense fallback={<PageFallback />}>
                    <Home />
                  </Suspense>
                ),
            },
            {
                path: 'clean',
                element: (
                  <Suspense fallback={<PageFallback />}>
                    <Clean />
                  </Suspense>
                ),
            },
            {
                path: 'uninstall',
                element: (
                  <Suspense fallback={<PageFallback />}>
                    <ShellUninstall />
                  </Suspense>
                ),
            },
            {
                path: 'optimize',
                element: (
                  <Suspense fallback={<PageFallback />}>
                    <Optimize />
                  </Suspense>
                ),
            },
            {
                path: 'analyze',
                element: (
                  <Suspense fallback={<PageFallback />}>
                    <Analyze />
                  </Suspense>
                ),
            },
        ]
    }
])
