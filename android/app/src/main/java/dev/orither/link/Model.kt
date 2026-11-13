package dev.orither.link

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement

@Serializable
data class Device(val name: String, val kind: String)

@Serializable
data class Peer(
    val id: String,
    val device: Device,
    @SerialName("paired_at") val pairedAt: Long,
    val online: Boolean = false,
    val route: String? = null,
)

@Serializable
data class Invite(
    val code: String,
    val uri: String,
    @SerialName("expires_in_secs") val expiresInSecs: Long,
)

/** What the Rust side reports. See `Update` in `orither-ffi`. */
@Serializable
internal sealed interface Update {
    @Serializable @SerialName("paired")
    data class Paired(val peer: Peer) : Update

    @Serializable @SerialName("unpaired")
    data class Unpaired(val peer: String) : Update

    @Serializable @SerialName("connected")
    data class Connected(val peer: Peer) : Update

    @Serializable @SerialName("disconnected")
    data class Disconnected(val peer: String) : Update

    @Serializable @SerialName("route")
    data class Route(val peer: String, val route: String) : Update

    @Serializable @SerialName("call")
    data class Call(
        val handle: Long,
        val peer: String,
        val feature: String,
        val method: String,
        val body: JsonElement,
    ) : Update

    @Serializable @SerialName("notice")
    data class Notice(
        val peer: String,
        val feature: String,
        val method: String,
        val body: JsonElement,
    ) : Update

    @Serializable @SerialName("stream")
    data class Stream(
        val handle: Long,
        val peer: String,
        val feature: String,
        val method: String,
        val body: JsonElement,
    ) : Update
}
