package dev.orither.feature

import dev.orither.link.LinkStream
import dev.orither.link.Peer
import kotlinx.serialization.json.JsonElement

/**
 * One thing the phone can do for the computer. A feature is addressed by [id] and
 * implements only what it needs. Adding one means writing the class and listing
 * it in [Features].
 */
interface Feature {
    val id: String

    /** What the user has to allow for this feature to work, if anything. */
    val access: Access? get() = null

    /** Answers a request from the computer. */
    suspend fun call(method: String, body: JsonElement, peer: String): JsonElement =
        throw UnsupportedOperationException("$id has no method $method")

    /** Handles a one-way message from the computer. */
    suspend fun notice(method: String, body: JsonElement, peer: String) {}

    /** Takes over a byte stream opened by the computer. */
    suspend fun stream(method: String, body: JsonElement, stream: LinkStream, peer: String) {
        stream.close()
    }

    fun connected(peer: Peer) {}

    fun disconnected(peer: String) {}

    /** The way to reach [peer] changed. [route] is one of local, nearby, internet, relay. */
    fun route(peer: String, route: String) {}

    /** The user just allowed something on the phone, maybe this feature's [access]. */
    fun allowed() {}
}

/** A feature used to check that the link works end to end. */
internal class Echo : Feature {
    override val id = "echo"

    override suspend fun call(method: String, body: JsonElement, peer: String): JsonElement = body
}

