import MainLayout from './layout';
import { createBrowserRouter, Navigate } from 'react-router-dom';
import { Home } from '@/pages/Home'
import { Clean } from '@/pages/Clean'
import { ShellUninstall } from '@/pages/Uninstall'
import { Optimize } from '@/pages/Optimize'
import { Analyze } from '@/pages/Analyze'
import { Dashboard } from '@/pages/Dashboard'
import { SystemConfirm } from '@/pages/SystemConfirm'

export const router = createBrowserRouter([
    {
        // 系统托盘仪表盘气泡：独立窗口（tauri.conf.json label=dashboard），
        // 不套 MainLayout（无侧边栏），顶层路由。
        path: '/dashboard',
        element: <Dashboard />,
    },
    {
        // 自定义原生确认弹窗：独立透明窗口（label=system-confirm），
        // 由 controllers/system_confirm.rs 的 mole_system_confirm 动态创建/复用。
        // 不套 MainLayout，无侧边栏与标题栏。
        path: '/system-confirm',
        element: <SystemConfirm />,
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
                element: <Home />,
            },
            {
                path: 'clean',
                element: <Clean />,
            },
            {
                path: 'uninstall',
                element: <ShellUninstall />,
            },
            {
                path: 'optimize',
                element: <Optimize />,
            },
            {
                path: 'analyze',
                element: <Analyze />,
            },
        ]
    }
])
