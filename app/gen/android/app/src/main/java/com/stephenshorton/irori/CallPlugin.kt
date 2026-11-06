package com.stephenshorton.irori

import android.Manifest
import android.app.Activity
import android.app.ForegroundServiceStartNotAllowedException
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.media.AudioAttributes
import android.media.AudioDeviceInfo
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.os.Build
import androidx.core.content.ContextCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class CallStartArgs {
  var title: String = ""
  lateinit var events: Channel
}

@InvokeArg
class CallRouteArgs {
  var to: String = "speaker"
}

// Card #654: the call's Android side. Rust's call::Platform (Phone, in
// app/src/shell/call_android.rs) calls start before the mic opens, stop after
// it closes, and route to pick the speaker or the earpiece. start runs
// CallService and puts the phone in its voice-call audio mode.
@TauriPlugin
class CallPlugin(private val activity: Activity) : Plugin(activity) {
  private var focus: AudioFocusRequest? = null

  private fun audio(): AudioManager =
    activity.getSystemService(Context.AUDIO_SERVICE) as AudioManager

  @Command
  fun start(invoke: Invoke) {
    val a = invoke.parseArgs(CallStartArgs::class.java)
    val out = JSObject()
    if (ContextCompat.checkSelfPermission(activity, Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
      out.put("ok", false)
      out.put("reason", "mic-denied")
      invoke.resolve(out)
      return
    }
    CallService.events = a.events
    val ctx = activity.applicationContext
    try {
      ContextCompat.startForegroundService(
        ctx, Intent(ctx, CallService::class.java).putExtra(CallService.EXTRA_TITLE, a.title)
      )
    } catch (e: Exception) {
      out.put("ok", false)
      // Android 12+ refuses a foreground service started from the background.
      if (Build.VERSION.SDK_INT >= 31 && e is ForegroundServiceStartNotAllowedException) {
        out.put("reason", "background")
      } else {
        out.put("reason", e.message ?: e.javaClass.simpleName)
      }
      invoke.resolve(out)
      return
    }
    val am = audio()
    am.mode = AudioManager.MODE_IN_COMMUNICATION
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && focus == null) {
      val attrs = AudioAttributes.Builder()
        .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
        .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
        .build()
      val req = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT)
        .setAudioAttributes(attrs)
        .build()
      am.requestAudioFocus(req)
      focus = req
    }
    // The volume keys set the call's volume, not media's.
    activity.volumeControlStream = AudioManager.STREAM_VOICE_CALL
    out.put("ok", true)
    invoke.resolve(out)
  }

  @Command
  fun stop(invoke: Invoke) {
    val ctx = activity.applicationContext
    ctx.stopService(Intent(ctx, CallService::class.java))
    CallService.events = null
    val am = audio()
    if (Build.VERSION.SDK_INT >= 31) {
      am.clearCommunicationDevice()
    } else {
      @Suppress("DEPRECATION")
      am.isSpeakerphoneOn = false
    }
    am.mode = AudioManager.MODE_NORMAL
    focus?.let { if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) am.abandonAudioFocusRequest(it) }
    focus = null
    activity.volumeControlStream = AudioManager.USE_DEFAULT_STREAM_TYPE
    invoke.resolve()
  }

  @Command
  fun route(invoke: Invoke) {
    val a = invoke.parseArgs(CallRouteArgs::class.java)
    val speaker = a.to == "speaker"
    val am = audio()
    val out = JSObject()
    if (Build.VERSION.SDK_INT >= 31) {
      val want = if (speaker) AudioDeviceInfo.TYPE_BUILTIN_SPEAKER else AudioDeviceInfo.TYPE_BUILTIN_EARPIECE
      val dev = am.availableCommunicationDevices.firstOrNull { it.type == want }
      out.put("ok", dev != null && am.setCommunicationDevice(dev))
    } else {
      @Suppress("DEPRECATION")
      am.isSpeakerphoneOn = speaker
      out.put("ok", true)
    }
    invoke.resolve(out)
  }
}
