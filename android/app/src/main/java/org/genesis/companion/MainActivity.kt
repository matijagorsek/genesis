package org.genesis.companion

import android.graphics.BitmapFactory
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

// Genesis colours, the same as the desktop
val Bg = Color(0xFF10161E); val Panel = Color(0xFF161D27); val Panel2 = Color(0xFF1D2631); val Line = Color(0xFF232C37)
val Ink = Color(0xFFE6EBF1); val Ink2 = Color(0xFFA9B4C1); val Ink3 = Color(0xFF8894A3); val Accent = Color(0xFF5FB5BD); val Good = Color(0xFF6CCF94); val Warn = Color(0xFFE0A84A); val Bad = Color(0xFFEF7B7B)

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // a pairing code handed over by intent (development, or a QR app that opens us): same as a scan
        intent?.getStringExtra("pair")?.let { Pairing.parse(it)?.save(this) }
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) != android.content.pm.PackageManager.PERMISSION_GRANTED)
            requestPermissions(arrayOf(android.Manifest.permission.POST_NOTIFICATIONS), 1)
        if (Pairing.load(this) != null) WatchService.start(this)
        setContent { GenesisTheme { App() } }
    }
}

@Composable fun GenesisTheme(content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = darkColorScheme(primary = Accent, background = Bg, surface = Panel, onBackground = Ink, onSurface = Ink, secondary = Ink2), content = content)
}

@Composable fun App() {
    val ctx = androidx.compose.ui.platform.LocalContext.current
    var pairing by remember { mutableStateOf(Pairing.load(ctx)) }
    Surface(Modifier.fillMaxSize(), color = Bg) {
        val p = pairing
        if (p == null) PairScreen { pairing = it; it.save(ctx); WatchService.start(ctx) }
        else Home(Genesis(p), p) { WatchService.stop(ctx); p.forget(ctx); pairing = null }
    }
}

@Composable fun PairScreen(onPaired: (Pairing) -> Unit) {
    var error by remember { mutableStateOf<String?>(null) }
    val scanner = androidx.activity.compose.rememberLauncherForActivityResult(ScanContract()) { res ->
        val text = res.contents ?: return@rememberLauncherForActivityResult
        val p = Pairing.parse(text)
        if (p == null) error = "That is not a Genesis pairing code." else onPaired(p)
    }
    Column(Modifier.fillMaxSize().padding(28.dp), verticalArrangement = Arrangement.Center) {
        Text("Genesis", color = Ink, fontSize = 34.sp, fontWeight = FontWeight.Bold)
        Spacer(Modifier.height(8.dp))
        Text("Your computer, from your phone: see what it is making, answer its questions, ask for something new. Same network, encrypted, nothing in between.", color = Ink2, fontSize = 15.sp, lineHeight = 22.sp)
        Spacer(Modifier.height(28.dp))
        Text("On the computer: Settings, Phone app. Then scan the code.", color = Ink3, fontSize = 13.sp)
        Spacer(Modifier.height(12.dp))
        Button(onClick = { scanner.launch(ScanOptions().setOrientationLocked(false).setBeepEnabled(false).setPrompt("Scan the code in Genesis Settings")) }) { Text("Scan the pairing code") }
        Spacer(Modifier.height(18.dp))
        var pasted by remember { mutableStateOf("") }
        Text("Or paste the code (Settings, Phone app, \"copy the code\"):", color = Ink3, fontSize = 13.sp)
        OutlinedTextField(pasted, { pasted = it }, Modifier.fillMaxWidth(), placeholder = { Text("genesis-pair:{…}", color = Ink3) }, minLines = 2)
        TextButton(enabled = pasted.isNotBlank(), onClick = { val p = Pairing.parse(pasted); if (p == null) error = "That is not a Genesis pairing code." else onPaired(p) }) { Text("Pair with the pasted code") }
        error?.let { Spacer(Modifier.height(12.dp)); Text(it, color = Bad) }
    }
}

@Composable fun Home(g: Genesis, p: Pairing, onForget: () -> Unit) {
    var tab by remember { mutableIntStateOf(0) }
    var sessions by remember { mutableStateOf(JSONArray()) }
    var health by remember { mutableStateOf<JSONObject?>(null) }
    var open by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    LaunchedEffect(Unit) {
        while (true) {
            val h = withContext(Dispatchers.IO) { g.health() }; health = h
            if (h.optInt("_status") == 200) sessions = withContext(Dispatchers.IO) { g.sessions() }
            delay(if (open != null) 2000 else 6000)
        }
    }
    val current = open
    if (current != null) { JobScreen(g, current, onBack = { open = null }); return }
    Scaffold(containerColor = Bg, bottomBar = {
        NavigationBar(containerColor = Panel) {
            NavigationBarItem(tab == 0, { tab = 0 }, { Text("Jobs") }, label = null)
            NavigationBarItem(tab == 1, { tab = 1 }, { Text("Make") }, label = null)
            NavigationBarItem(tab == 2, { tab = 2 }, { Text("Made") }, label = null)
            NavigationBarItem(tab == 3, { tab = 3 }, { Text("Screen") }, label = null)
            NavigationBarItem(tab == 4, { tab = 4 }, { Text("About") }, label = null)
        }
    }) { pad ->
        Column(Modifier.padding(pad).padding(18.dp).fillMaxSize()) {
            Header(p, health)
            Spacer(Modifier.height(14.dp))
            when (tab) {
                0 -> Jobs(sessions) { open = it }
                1 -> Make(g) { id -> open = id }
                2 -> MadeHere(g)
                3 -> Screen(g)
                else -> About(p, health, onForget)
            }
        }
    }
}

@Composable fun Header(p: Pairing, h: JSONObject?) {
    val ok = h?.optInt("_status") == 200
    val router = h?.optBoolean("router_ok") == true
    Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
        Text(p.name, color = Ink, fontSize = 22.sp, fontWeight = FontWeight.Bold)
        Spacer(Modifier.width(10.dp))
        Chip(if (!ok) "not reachable" else if (router) "on this machine" else "no models yet", if (!ok) Bad else if (router) Good else Warn)
    }
    h?.optJSONObject("os")?.optString("pretty")?.takeIf { it.isNotBlank() }?.let { Text(it, color = Ink3, fontSize = 12.sp) }
    if (!ok) h?.optString("error")?.takeIf { it.isNotBlank() }?.let { Text(it, color = Ink3, fontSize = 12.sp) }
}

@Composable fun Chip(text: String, color: Color) {
    Text(text, color = color, fontSize = 11.sp, fontFamily = FontFamily.Monospace, modifier = Modifier.background(Panel, RoundedCornerShape(999.dp)).padding(horizontal = 9.dp, vertical = 3.dp))
}

@Composable fun Jobs(sessions: JSONArray, onOpen: (String) -> Unit) {
    if (sessions.length() == 0) { Text("No jobs yet. Make something from the Make tab, or on the computer with Meta+Space.", color = Ink2); return }
    val items = (0 until sessions.length()).map { sessions.getJSONObject(it) }.reversed()
    LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        items(items) { s ->
            val state = s.optString("state"); val col = when (state) { "running" -> Accent; "waiting" -> Warn; "done" -> Good; "error" -> Bad; else -> Ink3 }
            Column(Modifier.fillMaxWidth().background(Panel, RoundedCornerShape(10.dp)).padding(12.dp).let { m -> m }.also { }.then(Modifier), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                    Text(s.optString("project").substringAfterLast('/').ifBlank { "home" }, color = Ink, fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
                    Chip(when (state) { "waiting" -> "needs you" ; else -> state }, col)
                }
                Text(s.optString("mode").replace('_', ' '), color = Ink3, fontSize = 12.sp)
                TextButton(onClick = { onOpen(s.getString("id")) }) { Text("Open") }
            }
        }
    }
}

@Composable fun Make(g: Genesis, onStarted: (String) -> Unit) {
    var text by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Text("What should Genesis make? It builds on the computer; steps that change things wait for your answer here or there.", color = Ink2, fontSize = 14.sp)
        OutlinedTextField(text, { text = it }, Modifier.fillMaxWidth(), placeholder = { Text("a checklist app that saves to a file", color = Ink3) }, minLines = 3)
        listOf("a pomodoro timer with a bell", "a word-count tool for text files", "a simple website for a club").forEach { s ->
            SuggestionChip(onClick = { text = s }, label = { Text(s, fontSize = 12.sp) })
        }
        Button(enabled = text.isNotBlank() && !busy, onClick = {
            busy = true; error = null
            scope.launch { val r = withContext(Dispatchers.IO) { g.make(text) }; busy = false
                if (r.optInt("_status") == 200 && r.has("id")) onStarted(r.getString("id")) else error = r.optString("error", "could not start") }
        }) { Text(if (busy) "Starting…" else "Make it") }
        error?.let { Text(it, color = Bad) }
    }
}

@Composable fun MadeHere(g: Genesis) {
    var list by remember { mutableStateOf(JSONArray()) }
    LaunchedEffect(Unit) { list = withContext(Dispatchers.IO) { g.made() } }
    if (list.length() == 0) { Text("Nothing made yet. Everything Genesis builds on the computer shows up here, with the words that asked for it.", color = Ink2); return }
    val items = (0 until list.length()).map { list.getJSONObject(it) }
    LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        items(items) { m ->
            Column(Modifier.fillMaxWidth().background(Panel, RoundedCornerShape(10.dp)).padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                    Text(m.optString("name"), color = Ink, fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
                    Chip(if (m.optBoolean("installed")) "in the app menu" else m.optString("template"), if (m.optBoolean("installed")) Good else Ink3)
                }
                val prompts = m.optJSONArray("prompts"); if (prompts != null && prompts.length() > 0) Text("\u201c" + prompts.getString(0) + "\u201d", color = Ink2, fontSize = 13.sp)
                Text(m.optString("made_at").replace('T', ' ').take(16), color = Ink3, fontSize = 11.sp)
            }
        }
    }
}

@Composable fun Screen(g: Genesis) {
    var img by remember { mutableStateOf<android.graphics.Bitmap?>(null) }
    var busy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Text("One picture of the computer's screen, taken when you ask. Never a stream.", color = Ink2, fontSize = 14.sp)
        Button(enabled = !busy, onClick = { busy = true; scope.launch { val b = withContext(Dispatchers.IO) { g.screenshot() }; img = b?.let { BitmapFactory.decodeByteArray(it, 0, it.size) }; busy = false } }) { Text(if (busy) "Taking…" else "Show me the screen") }
        img?.let { Image(it.asImageBitmap(), "The computer's screen", Modifier.fillMaxWidth()) }
    }
}

@Composable fun About(p: Pairing, h: JSONObject?, onForget: () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text("Paired with ${p.name} at ${p.hosts.joinToString(", ")}:${p.port}", color = Ink2)
        Text("Certificate ${p.fingerprint.take(16)}…, pinned. Only this machine's certificate is accepted.", color = Ink3, fontSize = 12.sp)
        h?.optString("model")?.takeIf { it.isNotBlank() }?.let { Text("Model: $it", color = Ink3, fontSize = 12.sp) }
        Spacer(Modifier.height(10.dp))
        OutlinedButton(onClick = onForget) { Text("Forget this computer") }
    }
}

@Composable fun JobScreen(g: Genesis, id: String, onBack: () -> Unit) {
    var s by remember { mutableStateOf<JSONObject?>(null) }
    var steer by remember { mutableStateOf("") }
    val scope = rememberCoroutineScope()
    LaunchedEffect(id) { while (true) { s = withContext(Dispatchers.IO) { g.session(id) }; delay(1500) } }
    Column(Modifier.fillMaxSize().padding(18.dp)) {
        Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
            TextButton(onClick = onBack) { Text("‹ Jobs") }
            Spacer(Modifier.weight(1f))
            val st = s?.optString("state") ?: "…"
            Chip(if (st == "waiting") "needs you" else st, when (st) { "running" -> Accent; "waiting" -> Warn; "done" -> Good; "error" -> Bad; else -> Ink3 })
        }
        val pending = s?.optJSONArray("pending") ?: JSONArray()
        for (i in 0 until pending.length()) {
            val pr = pending.getJSONObject(i)
            val (verb, detail) = plainVerb(pr.optString("tool"), pr.optJSONObject("args") ?: JSONObject())
            Column(Modifier.fillMaxWidth().background(Panel2, RoundedCornerShape(10.dp)).padding(12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text("Genesis wants to $verb", color = Ink, fontWeight = FontWeight.SemiBold)
                detail.takeIf { it.isNotBlank() }?.let { Text(it, color = Ink2, fontSize = 13.sp, fontFamily = FontFamily.Monospace) }
                pr.optString("reason").takeIf { it.isNotBlank() }?.let { Text(it.replace(Regex("^tier [A-Z0-9]+: "), ""), color = Ink3, fontSize = 12.sp) }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(onClick = { scope.launch { withContext(Dispatchers.IO) { g.answer(pr.getString("request_id"), true) } } }) { Text("Allow once") }
                    OutlinedButton(onClick = { scope.launch { withContext(Dispatchers.IO) { g.answer(pr.getString("request_id"), false) } } }) { Text("Not now") }
                }
            }
            Spacer(Modifier.height(8.dp))
        }
        val events = s?.optJSONArray("events") ?: JSONArray()
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            for (i in 0 until events.length()) {
                val e = events.getJSONObject(i); val kind = e.optString("kind", e.optString("type"))
                val text = when (kind) {
                    "user_prompt" -> "You: " + e.optString("text")
                    "assistant" -> e.optString("text")
                    "tool_call" -> "› " + e.optString("name") + " " + e.optString("args").take(120)
                    "tool_result" -> "  " + e.optString("summary").take(200)
                    "waiting" -> "? waiting for your answer: " + e.optString("name")
                    "resolved" -> if (e.optBoolean("allowed")) "· allowed" else "· not now"
                    "snapshot" -> "· snapshot " + e.optString("detail").take(80)
                    "decision" -> "· " + e.optString("verdict") + " " + e.optString("reason")
                    "error" -> "‼ " + e.optString("text")
                    "done" -> "done"
                    else -> e.toString().take(160)
                }
                val col = when (kind) { "assistant" -> Ink; "user_prompt" -> Accent; "error" -> Bad; "done" -> Good; else -> Ink3 }
                Text(text, color = col, fontSize = if (kind == "assistant" || kind == "user_prompt") 14.sp else 12.sp, fontFamily = if (kind == "tool_call" || kind == "tool_result") FontFamily.Monospace else FontFamily.Default)
            }
        }
        Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
            OutlinedTextField(steer, { steer = it }, Modifier.weight(1f), placeholder = { Text("Change something…", color = Ink3) })
            Spacer(Modifier.width(8.dp))
            Button(enabled = steer.isNotBlank(), onClick = { val t = steer; steer = ""; scope.launch { withContext(Dispatchers.IO) { g.steer(id, t) } } }) { Text("Send") }
        }
    }
}

/** The same plain verbs as the desktop's permission cards. */
fun plainVerb(tool: String, a: JSONObject): Pair<String, String> = when (tool) {
    "shell" -> { val c = a.optString("command").trim()
        if (Regex("\\b(pip3?|npm|dnf5?|flatpak|cargo|apt)\\b.*\\binstall\\b").containsMatchIn(c)) "install software" to c
        else if (a.optBoolean("needs_network")) "use the network" to c else "run a command" to c }
    "write_file", "edit_file" -> "change a file outside the project" to a.optString("path")
    "read_file", "list_dir" -> "read a file" to a.optString("path")
    "browser_open" -> "open a web page" to a.optString("url")
    "browser_click", "browser_type" -> "act on the web page" to a.optString("text")
    "install_app" -> "add an app to your menu" to a.optString("display_name")
    "preview_start" -> "run the project" to a.optString("path")
    "scaffold" -> "create the project ${a.optString("name")}" to "from the ${a.optString("template")} template"
    else -> tool.replace('_', ' ') to a.toString().take(120)
}
