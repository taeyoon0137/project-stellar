package stellar.generated

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.colorspace.ColorSpaces
import androidx.compose.ui.unit.dp

object StellarColors {
    val surface = Color(red = 0.96f, green = 0.96f, blue = 0.98f, alpha = 1f, colorSpace = ColorSpaces.DisplayP3)
    val accent = Color(red = 0f, green = 0.48f, blue = 1f, alpha = 1f, colorSpace = ColorSpaces.Srgb)
    val n00000000000000000000000000000001_4 = surface
    val n00000000000000000000000000000002_5 = Color(red = 0.7f, green = 0f, blue = 0f, alpha = 1f, colorSpace = ColorSpaces.Srgb)
    val n0000000000000000000000000000000c_4 = accent
    val n0000000000000000000000000000000d_4 = Color(red = 1.1f, green = 0.5f, blue = -0.05f, alpha = 1f, colorSpace = ColorSpaces.ExtendedSrgb)
}

class StellarState {
    var selectedSize by mutableStateOf("00000000-0000-0000-0000-000000000008")
    var memo by mutableStateOf("")
    var isSaving by mutableStateOf(false)
}

@Composable
fun StellarView(
    state: StellarState = remember { StellarState() },
    onAction: (String) -> Unit = {},
) {
    Node00000000000000000000000000000001(state, onAction)
}

@Composable
private fun Node00000000000000000000000000000001(state: StellarState, onAction: (String) -> Unit) {
    Column(
        modifier = Modifier
            .semantics { isTraversalGroup = true }
            .width(390.dp)
            .height(844.dp)
            .padding(start = 24.dp, top = 24.dp, end = 24.dp, bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp, Alignment.Top),
        horizontalAlignment = Alignment.Start,
    ) {
        Node00000000000000000000000000000002(state, onAction)
        Node00000000000000000000000000000004(state, onAction)
        Node00000000000000000000000000000005(state, onAction)
        Node00000000000000000000000000000006(state, onAction)
        Node0000000000000000000000000000000b(state, onAction)
        Node0000000000000000000000000000000c(state, onAction)
        Node0000000000000000000000000000000d(state, onAction)
        Node00000000000000000000000000000010(state, onAction)
    }
}

@Composable
private fun ColumnScope.Node00000000000000000000000000000002(state: StellarState, onAction: (String) -> Unit) {
    BasicText(
        text = "설정",
        modifier = Modifier.semantics { traversalIndex = 1f },
    )
}

@Composable
private fun ColumnScope.Node00000000000000000000000000000004(state: StellarState, onAction: (String) -> Unit) {
    Box(
        modifier = Modifier
            .semantics { role = Role.Image; contentDescription = "프로필 사진"; traversalIndex = 0f }
            .width(64.dp)
            .height(64.dp),
    )
}

@Composable
private fun ColumnScope.Node00000000000000000000000000000005(state: StellarState, onAction: (String) -> Unit) {
    Box(
        modifier = Modifier
            .clearAndSetSemantics {}
            .fillMaxWidth()
            .height(1.dp),
    )
}

@Composable
private fun ColumnScope.Node00000000000000000000000000000006(state: StellarState, onAction: (String) -> Unit) {
    Row(
        modifier = Modifier
            .selectableGroup()
            .semantics { contentDescription = "크기"; traversalIndex = 2f }
            .fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.Start),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Node00000000000000000000000000000007(state, onAction)
    }
}

@Composable
private fun RowScope.Node00000000000000000000000000000007(state: StellarState, onAction: (String) -> Unit) {
    Row(
        modifier = Modifier.weight(1f),
        horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.Start),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Node00000000000000000000000000000008(state, onAction)
        Node0000000000000000000000000000000a(state, onAction)
    }
}

@Composable
private fun RowScope.Node00000000000000000000000000000008(state: StellarState, onAction: (String) -> Unit) {
    Row(
        modifier = Modifier
            .weight(1f)
            .selectable(selected = state.selectedSize == "00000000-0000-0000-0000-000000000008", interactionSource = remember { MutableInteractionSource() }, indication = null, role = Role.RadioButton) { onAction("00000000-0000-0000-0000-00000000003d"); state.selectedSize = "00000000-0000-0000-0000-000000000008" }
            .height(IntrinsicSize.Max)
            .padding(start = 16.dp, top = 12.dp, end = 16.dp, bottom = 12.dp),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.Top,
    ) {
        Node00000000000000000000000000000009(state, onAction)
    }
}

@Composable
private fun RowScope.Node00000000000000000000000000000009(state: StellarState, onAction: (String) -> Unit) {
    BasicText(
        text = "작게",
        modifier = Modifier.fillMaxHeight(),
    )
}

@Composable
private fun RowScope.Node0000000000000000000000000000000a(state: StellarState, onAction: (String) -> Unit) {
    Box(
        modifier = Modifier
            .weight(1f)
            .selectable(selected = state.selectedSize == "00000000-0000-0000-0000-00000000000a", interactionSource = remember { MutableInteractionSource() }, indication = null, role = Role.RadioButton) { onAction("00000000-0000-0000-0000-00000000003e"); state.selectedSize = "00000000-0000-0000-0000-00000000000a" }
            .semantics(mergeDescendants = true) { contentDescription = "크게" }
            .padding(start = 16.dp, top = 12.dp, end = 16.dp, bottom = 12.dp),
    )
}

@Composable
private fun ColumnScope.Node0000000000000000000000000000000b(state: StellarState, onAction: (String) -> Unit) {
    BasicTextField(
        value = state.memo,
        onValueChange = { state.memo = it },
        modifier = Modifier
            .semantics { contentDescription = "메모"; traversalIndex = 3f }
            .fillMaxWidth()
            .height(44.dp)
            .padding(start = 12.dp, top = 8.dp, end = 12.dp, bottom = 8.dp),
        singleLine = true,
        decorationBox = { inner ->
            Box {
                if (state.memo.isEmpty()) BasicText(text = "메모")
                inner()
            }
        },
    )
}

@Composable
private fun ColumnScope.Node0000000000000000000000000000000c(state: StellarState, onAction: (String) -> Unit) {
    Box(
        modifier = Modifier
            .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, enabled = !(state.isSaving), role = Role.Button) { onAction("00000000-0000-0000-0000-00000000003c") }
            .semantics(mergeDescendants = true) { contentDescription = "저장"; traversalIndex = 4f }
            .widthIn(min = 120.dp)
            .fillMaxWidth()
            .height(48.dp),
    )
}

@Composable
private fun ColumnScope.Node0000000000000000000000000000000d(state: StellarState, onAction: (String) -> Unit) {
    Column(
        modifier = Modifier
            .semantics { traversalIndex = 5f }
            .fillMaxWidth(),
        verticalArrangement = Arrangement.Top,
    ) {
        Row(
            modifier = Modifier
                .height(IntrinsicSize.Max)
                .fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Box(
                modifier = Modifier
                    .weight(1f)
                    .fillMaxHeight(),
                contentAlignment = Alignment.TopStart,
            ) {
                Node0000000000000000000000000000000f(state, onAction)
            }
            Box(
                modifier = Modifier.weight(1f),
                contentAlignment = Alignment.TopStart,
            ) {
                Node00000000000000000000000000000012(state, onAction)
            }
        }
    }
}

@Composable
private fun BoxScope.Node0000000000000000000000000000000f(state: StellarState, onAction: (String) -> Unit) {
    BasicText(
        text = "새 기능",
        modifier = Modifier
            .fillMaxWidth()
            .fillMaxHeight(),
    )
}

@Composable
private fun BoxScope.Node00000000000000000000000000000012(state: StellarState, onAction: (String) -> Unit) {
    Box(
        modifier = Modifier
            .clearAndSetSemantics {}
            .width(24.dp)
            .height(24.dp),
    )
}

@Composable
private fun ColumnScope.Node00000000000000000000000000000010(state: StellarState, onAction: (String) -> Unit) {
    Box(
        modifier = Modifier
            .semantics { contentDescription = "알림"; paneTitle = "알림"; dismiss { onAction("00000000-0000-0000-0000-00000000003f"); true }; traversalIndex = 6f }
            .width(320.dp)
            .height(120.dp),
    ) {
        Node00000000000000000000000000000011(state, onAction)
    }
}

@Composable
private fun BoxScope.Node00000000000000000000000000000011(state: StellarState, onAction: (String) -> Unit) {
    BasicText(
        text = "저장되었습니다",
        modifier = Modifier
            .align(Alignment.TopStart)
            .offset(x = 16.dp, y = 16.dp),
    )
}
