package dev.orither.link

import android.os.ParcelFileDescriptor
import java.io.OutputStream
import kotlin.concurrent.thread
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.suspendCancellableCoroutine

/** A byte stream to the other device. Attach it to exactly one thing, or [close] it. */
class LinkStream internal constructor(private val handle: Long) {

    /** The local end of a new socket that carries the stream both ways. */
    fun socket(): ParcelFileDescriptor {
        val (mine, theirs) = ParcelFileDescriptor.createSocketPair()
        attach(theirs)
        return mine
    }

    /** Carries the stream both ways through an existing socket. Takes ownership of [socket]. */
    fun attach(socket: ParcelFileDescriptor) = Native.pump(handle, socket.detachFd())

    /**
     * Sends a capture's video from [socket], paced so that it stays live however slow
     * the link is. Only for a computer that asked for that. Takes ownership of [socket].
     */
    fun attachVideo(socket: ParcelFileDescriptor) = Native.pumpVideo(handle, socket.detachFd())

    /** Sends the rest of [file], then ends the stream. Takes ownership of [file]. */
    fun send(file: ParcelFileDescriptor) = Native.sendFile(handle, file.detachFd())

    /** Sends whatever [write] produces, on a thread of its own, then ends the stream. */
    fun send(write: (OutputStream) -> Unit) {
        val (reader, writer) = ParcelFileDescriptor.createPipe()
        send(reader)
        thread(isDaemon = true) {
            ParcelFileDescriptor.AutoCloseOutputStream(writer).use { runCatching { write(it) } }
        }
    }

    /** Writes the stream into [file] and returns the byte count. Takes ownership of [file]. */
    suspend fun receive(file: ParcelFileDescriptor): Long =
        suspendCancellableCoroutine { continuation ->
            Native.receiveFile(handle, file.detachFd()) { result, error ->
                if (result != null) {
                    continuation.resume(result.toLong())
                } else {
                    continuation.resumeWithException(LinkException(error.orEmpty()))
                }
            }
        }

    fun close() = Native.dropStream(handle)
}

class LinkException(message: String) : Exception(message)
