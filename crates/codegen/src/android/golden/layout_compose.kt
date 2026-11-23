@file:OptIn(ExperimentalLayoutApi::class)

package stellar.generated

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.FlowRowScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.text.BasicText
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.Matrix
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.unit.dp

object StellarColors {
}

class StellarState {
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
            .padding(start = 16.dp, top = 16.dp, end = 16.dp, bottom = 16.dp)
            .width(360.dp)
            .height(640.dp),
        verticalArrangement = Arrangement.SpaceBetween,
        horizontalAlignment = Alignment.Start,
    ) {
        Node00000000000000000000000000000002(state, onAction)
        Node00000000000000000000000000000005(state, onAction)
        Node00000000000000000000000000000008(state, onAction)
        Node0000000000000000000000000000000b(state, onAction)
        Node0000000000000000000000000000000e(state, onAction)
    }
}

@Composable
private fun ColumnScope.Node00000000000000000000000000000002(state: StellarState, onAction: (String) -> Unit) {
    FlowRow(
        modifier = Modifier
            .clearAndSetSemantics {}
            .fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(4.dp, Alignment.Start),
        verticalArrangement = Arrangement.Top,
    ) {
        Node00000000000000000000000000000003(state, onAction)
        Node00000000000000000000000000000004(state, onAction)
    }
}

@Composable
private fun FlowRowScope.Node00000000000000000000000000000003(state: StellarState, onAction: (String) -> Unit) {
    BasicText(
        text = "가",
        modifier = Modifier
            .alignByBaseline()
            .fillMaxWidth(0.5f),
    )
}

@Composable
private fun FlowRowScope.Node00000000000000000000000000000004(state: StellarState, onAction: (String) -> Unit) {
    BasicText(
        text = "나",
        modifier = Modifier.alignByBaseline(),
    )
}

@Composable
private fun ColumnScope.Node00000000000000000000000000000005(state: StellarState, onAction: (String) -> Unit) {
    Row(
        modifier = Modifier
            .clearAndSetSemantics {}
            .fillMaxWidth()
            .height(40.dp),
        horizontalArrangement = Arrangement.SpaceAround,
        verticalAlignment = Alignment.Top,
    ) {
        Node00000000000000000000000000000007(state, onAction)
        Node00000000000000000000000000000006(state, onAction)
    }
}

@Composable
private fun RowScope.Node00000000000000000000000000000006(state: StellarState, onAction: (String) -> Unit) {
    Box(
        modifier = Modifier
            .weight(2f)
            .fillMaxHeight(),
    )
}

@Composable
private fun RowScope.Node00000000000000000000000000000007(state: StellarState, onAction: (String) -> Unit) {
    Box(
        modifier = Modifier
            .weight(1f)
            .fillMaxHeight(),
    )
}

@Composable
private fun ColumnScope.Node00000000000000000000000000000008(state: StellarState, onAction: (String) -> Unit) {
    Column(
        modifier = Modifier
            .clearAndSetSemantics {}
            .fillMaxWidth(),
        verticalArrangement = Arrangement.Top,
    ) {
        Row(
            modifier = Modifier
                .height(40.dp)
                .fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Box(
                modifier = Modifier
                    .width(100.dp)
                    .fillMaxHeight(),
                contentAlignment = Alignment.TopStart,
            ) {
                Node00000000000000000000000000000009(state, onAction)
            }
            Box(
                modifier = Modifier
                    .weight(1f)
                    .fillMaxHeight(),
                contentAlignment = Alignment.TopStart,
            ) {
                Node0000000000000000000000000000000a(state, onAction)
            }
            Spacer(Modifier.weight(2f))
        }
    }
}

@Composable
private fun BoxScope.Node00000000000000000000000000000009(state: StellarState, onAction: (String) -> Unit) {
    BasicText(
        text = "넓은 칸",
        modifier = Modifier
            .fillMaxWidth()
            .fillMaxHeight(),
    )
}

@Composable
private fun BoxScope.Node0000000000000000000000000000000a(state: StellarState, onAction: (String) -> Unit) {
    BasicText(
        text = "자동",
        modifier = Modifier
            .fillMaxWidth()
            .fillMaxHeight(),
    )
}

@Composable
private fun ColumnScope.Node0000000000000000000000000000000b(state: StellarState, onAction: (String) -> Unit) {
    Box(
        modifier = Modifier
            .offset(x = 8.dp, y = 4.dp)
            .drawWithContent { withTransform({ transform(Matrix(floatArrayOf(0f, 1f, 0f, 0f, -1f, 0f, 0f, 0f, 0f, 0f, 1f, 0f, 0f, 0f, 0f, 1f))) }) { this@drawWithContent.drawContent() } }
            .clearAndSetSemantics {}
            .width(160.dp)
            .aspectRatio(16f / 9f),
    )
}

@Composable
private fun ColumnScope.Node0000000000000000000000000000000e(state: StellarState, onAction: (String) -> Unit) {
    Box(
        modifier = Modifier
            .clearAndSetSemantics {}
            .width(200.dp)
            .height(100.dp),
    ) {
        Node0000000000000000000000000000000f(state, onAction)
        Node00000000000000000000000000000010(state, onAction)
    }
}

@Composable
private fun BoxScope.Node0000000000000000000000000000000f(state: StellarState, onAction: (String) -> Unit) {
    Box(
        modifier = Modifier
            .align(Alignment.BottomEnd)
            .offset(x = (-8).dp, y = (-8).dp)
            .width(20.dp)
            .height(20.dp),
    )
}

@Composable
private fun BoxScope.Node00000000000000000000000000000010(state: StellarState, onAction: (String) -> Unit) {
    Box(
        modifier = Modifier
            .align(Alignment.TopStart)
            .width(10.dp)
            .height(10.dp),
    )
}
