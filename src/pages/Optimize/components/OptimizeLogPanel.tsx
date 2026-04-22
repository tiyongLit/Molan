import { useEffect, useRef } from 'react'
import classnames from 'classnames'
import { useI18n } from '@/i18n'
import './OptimizeLogPanel.css'

export interface OptimizeLogLine {
  ts: number
  /** meta=阶段标记 / info=普通 / ok=成功 / fail=失败 */
  level: 'meta' | 'info' | 'ok' | 'fail'
  text: string
}

export interface OptimizeLogPanelProps {
  logs: OptimizeLogLine[]
  expanded: boolean
  onToggle: () => void
}

const LEVEL_COLOR: Record<OptimizeLogLine['level'], string> = {
  meta: 'rgba(140, 180, 220, 0.95)',
  info: 'rgba(255, 255, 255, 0.6)',
  ok: 'rgba(51, 211, 157, 0.95)',
  fail: 'rgba(252, 165, 165, 0.95)',
}

/**
 * 执行阶段日志面板（对齐 V1 Optimize 的日志盒）。
 * 折叠时只显示一行开关；展开时显示固定高度滚动区 + 自动滚到底部。
 */
export default function OptimizeLogPanel({ logs, expanded, onToggle }: OptimizeLogPanelProps) {
  const { locale, t } = useI18n()
  const boxRef = useRef<HTMLDivElement | null>(null)

  useEffect(() => {
    if (!expanded || !boxRef.current) return
    boxRef.current.scrollTop = boxRef.current.scrollHeight
  }, [logs, expanded])

  return (
    <div className="shrink-0 border-t border-white/[0.12] bg-black/30">
      <button
        onClick={onToggle}
        className="w-full flex items-center gap-2 px-[52px] py-1.5 text-left hover:bg-white/[0.04] transition-colors"
      >
        <span
          className={classnames('text-[10px] transition-transform', expanded ? 'rotate-90' : '')}
        >
          ▸
        </span>
        <span className="text-[10px] font-medium text-white/50">{t('optimize.log.title')}</span>
        <span className="text-[10px] text-white/30">{t('optimize.log.lines', { count: logs.length })}</span>
      </button>

      <div
        className={classnames(
          'grid overflow-hidden transition-[grid-template-rows,opacity] duration-250 ease-[cubic-bezier(0.4,0,0.2,1)]',
          expanded ? 'opacity-100' : 'opacity-0'
        )}
        style={{ gridTemplateRows: expanded ? '1fr' : '0fr' }}
      >
        <div className="min-h-0">
          <div ref={boxRef} className="optimize-log-box">
            {logs.map((l, i) => (
              <div key={`${l.ts}-${i}`} className="optimize-log-line">
                <span className="optimize-log-ts">{new Date(l.ts).toLocaleTimeString(locale, { hour12: false })}</span>
                <span style={{ color: LEVEL_COLOR[l.level] }}>{l.text}</span>
              </div>
            ))}
            {logs.length === 0 && <div className="optimize-log-line"><span style={{ color: 'rgba(255,255,255,0.3)' }}>{t('optimize.log.waiting')}</span></div>}
          </div>
        </div>
      </div>
    </div>
  )
}
