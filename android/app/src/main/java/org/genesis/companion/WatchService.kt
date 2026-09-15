package org.genesis.companion

import android.app.*
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder
import kotlinx.coroutines.*
import org.json.JSONObject

/**
 * Keeps an eye on the paired Genesis while the app is not in front: when a job waits for your answer,
 * a notification with Allow once / Not now appears. A small, quiet foreground service (Android requires
 * one for anything that keeps running); it stops itself when the phone is unpaired.
 */
class WatchService : Service() {
    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    private val notified = HashSet<String>()

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val pairing = Pairing.load(this) ?: run { stopSelf(); return START_NOT_STICKY }
        channels()
        startForeground(1, quiet("Connected to ${pairing.name}", "Watching for jobs that need you"))
        val g = Genesis(pairing)
        scope.launch {
            while (isActive) {
                try {
                    val list = g.sessions()
                    android.util.Log.i("GenesisWatch", "poll: ${list.length()} sessions via ${g.host}")
                    for (i in 0 until list.length()) {
                        val s = list.getJSONObject(i)
                        val id = s.optString("id")
                        if (s.optString("state") == "waiting") {
                            val info = g.session(id)
                            val pending = info.optJSONArray("pending")
                            if (pending != null && pending.length() > 0) {
                                val pr = pending.getJSONObject(0)
                                val rid = pr.optString("request_id")
                                if (notified.add(rid)) ask(id, rid, pr)
                            }
                        } else if (s.optString("state") == "done") {
                            if (notified.add("done:$id")) done(id, s.optString("project").substringAfterLast('/'))
                        }
                    }
                } catch (e: Exception) { android.util.Log.w("GenesisWatch", "poll failed", e) }
                delay(8000)
            }
        }
        return START_STICKY
    }

    override fun onDestroy() { scope.cancel(); super.onDestroy() }

    private fun channels() {
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel("watch", "Connection", NotificationManager.IMPORTANCE_MIN))
        nm.createNotificationChannel(NotificationChannel("ask", "Genesis needs you", NotificationManager.IMPORTANCE_HIGH))
        nm.createNotificationChannel(NotificationChannel("done", "Job finished", NotificationManager.IMPORTANCE_DEFAULT))
    }

    private fun quiet(title: String, text: String): Notification =
        Notification.Builder(this, "watch").setSmallIcon(android.R.drawable.ic_menu_view).setContentTitle(title).setContentText(text).setOngoing(true).build()

    private fun open(): PendingIntent = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)

    private fun ask(sessionId: String, requestId: String, pr: JSONObject) {
        val (verb, detail) = plainVerb(pr.optString("tool"), pr.optJSONObject("args") ?: JSONObject())
        fun act(allow: Boolean) = PendingIntent.getBroadcast(this, (requestId.hashCode() * 2) + (if (allow) 1 else 0),
            Intent(this, AnswerReceiver::class.java).putExtra("request_id", requestId).putExtra("allow", allow), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = Notification.Builder(this, "ask").setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle("Genesis wants to $verb").setContentText(detail.ifBlank { "Open to see the job" }).setContentIntent(open()).setAutoCancel(true)
            .addAction(Notification.Action.Builder(null, "Allow once", act(true)).build())
            .addAction(Notification.Action.Builder(null, "Not now", act(false)).build()).build()
        getSystemService(NotificationManager::class.java).notify(requestId.hashCode(), n)
    }

    private fun done(id: String, name: String) {
        val n = Notification.Builder(this, "done").setSmallIcon(android.R.drawable.ic_menu_upload).setContentTitle("Genesis finished")
            .setContentText(if (name.isBlank()) "A job is done" else "$name is done").setContentIntent(open()).setAutoCancel(true).build()
        getSystemService(NotificationManager::class.java).notify(id.hashCode(), n)
    }

    companion object {
        fun start(ctx: Context) {
            val i = Intent(ctx, WatchService::class.java)
            if (Build.VERSION.SDK_INT >= 26) ctx.startForegroundService(i) else ctx.startService(i)
        }
        fun stop(ctx: Context) { ctx.stopService(Intent(ctx, WatchService::class.java)) }
    }
}

/** The Allow / Not now buttons on the notification. */
class AnswerReceiver : BroadcastReceiver() {
    override fun onReceive(ctx: Context, intent: Intent) {
        val rid = intent.getStringExtra("request_id") ?: return
        val allow = intent.getBooleanExtra("allow", false)
        val pairing = Pairing.load(ctx) ?: return
        ctx.getSystemService(NotificationManager::class.java).cancel(rid.hashCode())
        val pending = goAsync()
        Thread { try { Genesis(pairing).answer(rid, allow) } catch (_: Exception) {} finally { pending.finish() } }.start()
    }
}
