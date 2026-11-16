package dev.orither.link

import android.content.Context

/** The functions implemented by the `orither-ffi` Rust crate. Bodies are JSON text. */
internal object Native {
    init {
        System.loadLibrary("orither")
    }

    fun interface Listener {
        fun onEvent(json: String)
    }

    fun interface Reply {
        fun onReply(result: String?, error: String?)
    }

    external fun start(context: Context, dataDir: String, name: String, listener: Listener): String
    external fun peers(): String
    external fun invite(): String
    external fun cancelInvite()
    external fun join(token: String, reply: Reply)
    external fun unpair(peer: String)
    external fun networkChanged()
    external fun call(peer: String, feature: String, method: String, body: String, reply: Reply)
    external fun notify(peer: String, feature: String, method: String, body: String)
    external fun respond(handle: Long, ok: Boolean, body: String)
    external fun open(peer: String, feature: String, method: String, body: String, reply: Reply)
    external fun pump(handle: Long, socket: Int)
    external fun pumpVideo(handle: Long, socket: Int)
    external fun addNearby(socket: Int, reply: Reply)
    external fun sendFile(handle: Long, file: Int)
    external fun receiveFile(handle: Long, file: Int, reply: Reply)
    external fun dropStream(handle: Long)
}
