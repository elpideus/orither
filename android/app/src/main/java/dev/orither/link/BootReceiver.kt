package dev.orither.link

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** Brings the link back after the phone restarts or the app updates. */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) = LinkService.start(context)
}
