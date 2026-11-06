package com.stephenshorton.irori

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import app.tauri.plugin.Channel
import app.tauri.plugin.JSObject

// Card #654: keeps a call running while the screen is off or Irori is in the
// background. A foreground service of type microphone, with an ongoing
// notification whose Hang up tells Rust, and a partial wake lock. CallPlugin
// starts and stops it; Rust's events channel hears hangup and failed.
class CallService : Service() {
  private var wake: PowerManager.WakeLock? = null

  override fun onBind(intent: Intent?): IBinder? = null

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    if (intent?.action == ACTION_HANGUP) {
      hangUp()
      return START_NOT_STICKY
    }
    val title = intent?.getStringExtra(EXTRA_TITLE) ?: "In a call"
    try {
      ServiceCompat.startForeground(
        this, NOTIFICATION_ID, notification(title),
        if (Build.VERSION.SDK_INT >= 30) ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE else 0
      )
    } catch (e: Exception) {
      // Android 14+ throws SecurityException here without RECORD_AUDIO, or
      // when the app wasn't on screen; uncaught it would crash the app.
      fail(e.message ?: e.javaClass.simpleName)
      return START_NOT_STICKY
    }
    running = true
    lastFailure = null
    if (wake == null) {
      val pm = getSystemService(POWER_SERVICE) as PowerManager
      wake = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "irori:call").apply {
        setReferenceCounted(false)
        acquire()
      }
    }
    return START_NOT_STICKY
  }

  // Swiping Irori away from recents ends the call, as Hang up does.
  override fun onTaskRemoved(rootIntent: Intent?) {
    super.onTaskRemoved(rootIntent)
    hangUp()
  }

  override fun onDestroy() {
    wake?.let { if (it.isHeld) it.release() }
    wake = null
    running = false
    super.onDestroy()
  }

  private fun hangUp() {
    send("hangup", null)
    stopSelf()
  }

  private fun fail(reason: String) {
    lastFailure = reason
    send("failed", reason)
    stopSelf()
  }

  private fun notification(title: String): Notification {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      // Low importance: the notification stays up without a sound.
      val ch = NotificationChannel(CHANNEL, "Calls", NotificationManager.IMPORTANCE_LOW)
      ch.description = "Shown while you're in a call or huddle"
      getSystemService(NotificationManager::class.java).createNotificationChannel(ch)
    }
    val open = PendingIntent.getActivity(
      this, 0,
      Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
    )
    val hang = PendingIntent.getService(
      this, 1,
      Intent(this, CallService::class.java).setAction(ACTION_HANGUP),
      PendingIntent.FLAG_IMMUTABLE
    )
    return NotificationCompat.Builder(this, CHANNEL)
      .setSmallIcon(R.drawable.ic_notify)
      .setContentTitle(title)
      .setOngoing(true)
      .setCategory(NotificationCompat.CATEGORY_CALL)
      .setPriority(NotificationCompat.PRIORITY_LOW)
      .setUsesChronometer(true)
      .setWhen(System.currentTimeMillis())
      .setContentIntent(open)
      .addAction(0, "Hang up", hang)
      .build()
  }

  companion object {
    const val CHANNEL = "call"
    const val NOTIFICATION_ID = 654
    const val EXTRA_TITLE = "irori.call.title"
    const val ACTION_HANGUP = "com.stephenshorton.irori.call.HANGUP"

    // Rust's events channel; CallPlugin sets it before each start.
    @Volatile var events: Channel? = null
    @Volatile var running = false
    @Volatile var lastFailure: String? = null

    fun send(event: String, reason: String?) {
      val o = JSObject()
      o.put("event", event)
      if (reason != null) o.put("reason", reason)
      try {
        events?.send(o)
      } catch (e: Exception) {
        // Rust is gone with the process; nothing to tell.
      }
    }
  }
}
