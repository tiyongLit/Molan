import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { EVT_TRASH_REMINDER_STATE } from '@/constants/tauri-events'
import { CMD_MOLE_TRASH_EMPTY, CMD_MOLE_TRASH_REMINDER_ACTION, CMD_MOLE_TRASH_REMINDER_GET_STATE } from '@/constants/tauri-commands'
import type { TrashEmptyResult, TrashReminderAction, TrashReminderSnapshot } from '@/types/mole'
import { t, trashReminderError } from '@/i18n'
import { moleNativeConfirm } from '@/hooks/useMoleConfirm'
import brandIcon from '../../../src-tauri/icons/icon.png'
import styles from './TrashReminder.module.scss'

export function TrashReminderWindow() {
  const [snapshot, setSnapshot] = useState<TrashReminderSnapshot | null>(null)
  const [visible, setVisible] = useState(false)
  const [pending, setPending] = useState(false)
  const [localError, setLocalError] = useState('')
  const latest = useRef<TrashReminderSnapshot | null>(null)
  const busy = useRef(false)
  const mounted = useRef(false)
  const operation = useRef(0)

  const apply = useCallback((next: TrashReminderSnapshot) => {
    if (!mounted.current || next.revision < (latest.current?.revision ?? -1)) return
    if (next.reminderId !== latest.current?.reminderId) {
      operation.current += 1
      busy.current = false
      setPending(false)
      setLocalError('')
    }
    latest.current = next
    setSnapshot(next)
  }, [])

  const action = useCallback((kind: TrashReminderAction, snap: TrashReminderSnapshot) =>
    invoke<TrashReminderSnapshot>(CMD_MOLE_TRASH_REMINDER_ACTION, {
      args: { action: kind, reminderId: snap.reminderId, revision: snap.revision },
    }), [])

  const snooze = useCallback(async () => {
    const snap = latest.current
    if (!snap?.reminderId || busy.current) return
    busy.current = true
    const token = ++operation.current
    const current = () => mounted.current && operation.current === token
    setPending(true)
    setLocalError('')
    try { const next = await action('snooze', snap); if (current()) apply(next) }
    catch (error) { if (current()) setLocalError(trashReminderError(String(error))) }
    finally { if (current()) { busy.current = false; setPending(false) } }
  }, [action, apply])

  useEffect(() => {
    mounted.current = true
    let disposed = false
    let unlisten: UnlistenFn | undefined
    let retry: ReturnType<typeof setTimeout> | undefined
    document.documentElement.style.background = 'transparent'
    document.body.style.background = 'transparent'
    const root = document.getElementById('root')
    if (root) { root.style.background = 'transparent'; root.style.overflow = 'hidden' }
    const reconcile = () => {
      if (!disposed) invoke<TrashReminderSnapshot>(CMD_MOLE_TRASH_REMINDER_GET_STATE)
        .then(next => { if (!disposed) apply(next) })
        .catch(error => console.warn('[trash-reminder] reconcile', error))
    }
    const subscribe = async () => {
      try {
        const off = await listen<TrashReminderSnapshot>(EVT_TRASH_REMINDER_STATE, e => {
          if (!disposed) apply(e.payload)
        })
        if (disposed) { off(); return }
        unlisten = off
        reconcile()
      } catch (error) {
        console.warn('[trash-reminder] subscribe', error)
        if (!disposed) retry = setTimeout(() => void subscribe(), 1000)
      }
    }
    void subscribe()
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); void snooze() } }
    const onFocus = () => { if (unlisten) reconcile() }
    const onVisible = () => { if (document.visibilityState === 'visible') onFocus() }
    window.addEventListener('keydown', onKey)
    window.addEventListener('focus', onFocus)
    document.addEventListener('visibilitychange', onVisible)
    return () => {
      disposed = true
      mounted.current = false
      operation.current += 1
      busy.current = false
      clearTimeout(retry)
      unlisten?.()
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('focus', onFocus)
      document.removeEventListener('visibilitychange', onVisible)
    }
  }, [apply, snooze])

  // DOM 已提交后才展示原生窗口；每个回调绑定版本，后端再次校验后才 show/hide。
  useEffect(() => {
    if (!snapshot) return
    let stale = false
    let frame = 0
    let timer: ReturnType<typeof setTimeout> | undefined
    const current = () => !stale && latest.current?.revision === snapshot.revision
    if (snapshot.state === 'hidden') {
      setVisible(false)
      setLocalError('')
      const delay = matchMedia('(prefers-reduced-motion: reduce)').matches ? 0 : 160
      timer = setTimeout(() => {
        if (current()) void action('hide', snapshot).catch(error => console.warn('[trash-reminder] hide', error))
      }, delay)
    } else {
      void (async () => {
        const shown = await action('show', snapshot)
        if (shown.revision !== snapshot.revision) { apply(shown); return }
        if (!current()) return
        frame = requestAnimationFrame(() => {
          if (!current()) return
          setVisible(true)
          if (snapshot.state === 'candidate') {
            void action('shown', snapshot).then(apply).catch(error => console.warn('[trash-reminder] ack', error))
          }
        })
      })().catch(error => { if (current()) console.warn('[trash-reminder] show', error) })
    }
    return () => { stale = true; cancelAnimationFrame(frame); clearTimeout(timer) }
  }, [snapshot, action, apply])

  // 主按钮：二次确认后清空废纸篓（永久删除，已用户确认）。后端硬编码 ~/.Trash，
  // 成功后会发布 Hidden 快照，卡片随即淡出；仅部分失败时保留卡片诚实提示。
  const emptyTrash = async () => {
    const snap = latest.current
    if (!snap?.reminderId || busy.current) return
    busy.current = true
    const token = ++operation.current
    const current = () => mounted.current && operation.current === token
    setPending(true)
    setLocalError('')
    try {
      const confirmed = await moleNativeConfirm(t('trashReminder.emptyConfirmTitle'), {
        informativeText: t('trashReminder.emptyConfirmBody'),
        kind: 'warning',
        okLabel: t('trashReminder.empty'),
        cancelLabel: t('trashReminder.cancel'),
      })
      if (!confirmed) return
      const result = await invoke<TrashEmptyResult>(CMD_MOLE_TRASH_EMPTY)
      if (current() && result.failed > 0) setLocalError(t('trashReminder.emptyPartialFailed'))
    } catch (error) {
      if (current()) setLocalError(trashReminderError(String(error)))
    } finally { if (current()) { busy.current = false; setPending(false) } }
  }

  const threshold = snapshot?.thresholdMB ?? 1024
  const label = threshold >= 1024 ? `${threshold / 1024} GB` : `${threshold} MB`
  const working = pending
  const error = localError || (snapshot?.errorCode ? trashReminderError(snapshot.errorCode) : '')
  return (
    <div className={styles.shell}>
      <section className={`${styles.card} ${visible ? styles.visible : ''}`} aria-label={t('trashReminder.title')}
        aria-hidden={!visible} onMouseDown={e => {
          if (e.button === 0 && e.target === e.currentTarget) void getCurrentWindow().startDragging()
        }}>
        <img className={styles.icon} src={brandIcon} alt="" draggable={false} />
        <div className={styles.copy} role="status" aria-live="polite">
          <h1 className={styles.title}>{t('trashReminder.title')}</h1>
          <p className={styles.description}>{error || <>{t('trashReminder.capacity')} <span className={styles.threshold}>{label}</span></>}</p>
        </div>
        <div className={styles.actions}>
          <button className={styles.secondary} disabled={working || !visible} onClick={() => void snooze()}>{t('trashReminder.snooze')}</button>
          <button className={styles.primary} disabled={working || !visible} onClick={() => void emptyTrash()}>
            {t(working ? 'trashReminder.emptying' : error ? 'trashReminder.retry' : 'trashReminder.empty')}
          </button>
        </div>
      </section>
    </div>
  )
}
