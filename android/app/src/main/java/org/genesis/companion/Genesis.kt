package org.genesis.companion

import android.content.Context
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject
import java.security.MessageDigest
import java.security.cert.X509Certificate
import java.util.concurrent.TimeUnit
import javax.net.ssl.SSLContext
import javax.net.ssl.X509TrustManager

/** What the QR in Genesis Settings carries: address(es), port, certificate fingerprint, pairing secret. */
data class Pairing(val name: String, val hosts: List<String>, val port: Int, val fingerprint: String, val token: String) {
    companion object {
        const val PREFIX = "genesis-pair:"
        fun parse(text: String): Pairing? {
            val raw = text.removePrefix(PREFIX).trim()
            if (!raw.startsWith("{")) return null
            return try {
                val o = JSONObject(raw)
                val hosts = o.optJSONArray("hosts")?.let { a -> (0 until a.length()).map { a.getString(it) } } ?: emptyList()
                Pairing(o.optString("name", "genesis"), hosts, o.optInt("port", 11530), o.getString("fp").lowercase(), o.getString("token"))
            } catch (e: Exception) { null }
        }
        fun load(ctx: Context): Pairing? {
            val p = ctx.getSharedPreferences("pairing", Context.MODE_PRIVATE)
            val fp = p.getString("fp", null) ?: return null
            return Pairing(p.getString("name", "genesis")!!, p.getString("hosts", "")!!.split(",").filter { it.isNotBlank() }, p.getInt("port", 11530), fp, p.getString("token", "")!!)
        }
    }
    fun save(ctx: Context) {
        ctx.getSharedPreferences("pairing", Context.MODE_PRIVATE).edit().putString("name", name).putString("hosts", hosts.joinToString(",")).putInt("port", port).putString("fp", fingerprint).putString("token", token).apply()
    }
    fun forget(ctx: Context) { ctx.getSharedPreferences("pairing", Context.MODE_PRIVATE).edit().clear().apply() }
}

/** The one thing the app trusts: the certificate whose SHA-256 was in the QR code. Nothing else, no CA. */
class PinnedTrust(private val fingerprint: String) : X509TrustManager {
    override fun checkClientTrusted(chain: Array<X509Certificate>, authType: String) = throw java.security.cert.CertificateException("no client certs")
    override fun checkServerTrusted(chain: Array<X509Certificate>, authType: String) {
        val leaf = chain.firstOrNull() ?: throw java.security.cert.CertificateException("empty chain")
        val fp = MessageDigest.getInstance("SHA-256").digest(leaf.encoded).joinToString("") { "%02x".format(it) }
        if (fp != fingerprint) throw java.security.cert.CertificateException("this is not the Genesis machine you paired with")
    }
    override fun getAcceptedIssuers(): Array<X509Certificate> = arrayOf()
}

/** Talks to genesis-companiond: tries every address from the QR, keeps the first that answers. */
class Genesis(private val pairing: Pairing) {
    private val client: OkHttpClient
    @Volatile var host: String? = null
    init {
        val tm = PinnedTrust(pairing.fingerprint)
        val ctx = SSLContext.getInstance("TLS"); ctx.init(null, arrayOf(tm), null)
        client = OkHttpClient.Builder().sslSocketFactory(ctx.socketFactory, tm).hostnameVerifier { _, _ -> true }
            .connectTimeout(4, TimeUnit.SECONDS).readTimeout(40, TimeUnit.SECONDS).build()
    }
    private fun url(path: String, h: String) = "https://$h:${pairing.port}$path"
    private fun exec(path: String, body: String? = null): JSONObject {
        val hosts = listOfNotNull(host) + pairing.hosts.filter { it != host }
        var last: Exception? = null
        for (h in hosts) {
            try {
                val b = Request.Builder().url(url(path, h)).header("Authorization", "Bearer ${pairing.token}")
                if (body != null) b.post(body.toRequestBody("application/json".toMediaType()))
                client.newCall(b.build()).execute().use { r ->
                    host = h
                    val text = r.body?.string() ?: "{}"
                    val json = if (text.trim().startsWith("[")) JSONObject().put("list", JSONArray(text)) else JSONObject(text)
                    json.put("_status", r.code)
                    return json
                }
            } catch (e: Exception) { last = e }
        }
        return JSONObject().put("error", "Genesis not reachable on this network (${last?.message ?: "no address"})").put("_status", 0)
    }
    fun health() = exec("/v1/health")
    fun sessions(): JSONArray = exec("/v1/sessions").optJSONArray("list") ?: JSONArray()
    fun session(id: String) = exec("/v1/sessions/$id")
    fun notices() = exec("/v1/notices")
    fun make(text: String) = exec("/v1/sessions", JSONObject().put("text", text).toString())
    fun steer(id: String, text: String) = exec("/v1/sessions/$id/prompt", JSONObject().put("text", text).toString())
    fun answer(promptId: String, allow: Boolean) = exec("/v1/prompts/$promptId", JSONObject().put("allow", allow).toString())
    fun screenshot(): ByteArray? {
        val hosts = listOfNotNull(host) + pairing.hosts.filter { it != host }
        for (h in hosts) {
            try {
                val req = Request.Builder().url(url("/v1/screenshot", h)).header("Authorization", "Bearer ${pairing.token}").build()
                client.newCall(req).execute().use { r -> if (r.isSuccessful) return r.body?.bytes() }
            } catch (_: Exception) {}
        }
        return null
    }
}
