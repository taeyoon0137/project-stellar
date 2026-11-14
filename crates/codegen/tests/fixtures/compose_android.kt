package com.example.generated

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.absolutePadding
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.isTraversalGroup
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.traversalIndex
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog

@Composable
fun SettingsView(onAction: (String) -> Unit = {}) {
    var selectedSize by remember { mutableStateOf("00000000-0000-0000-0000-000000000008") }
    var memo by remember { mutableStateOf("") }
    var isSaving by remember { mutableStateOf(false) }
    Column(
        modifier = Modifier
            .size(390.dp, 844.dp)
            .semantics {
                isTraversalGroup = true
            }
            .padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp, Alignment.Top),
        horizontalAlignment = Alignment.Start,
    ) {
        BasicText(
            modifier = Modifier
                .fillMaxWidth()
                .semantics {
                    traversalIndex = 1f
                },
            text = "설정",
        )
        Box(
            modifier = Modifier
                .size(64.dp, 64.dp)
                .semantics {
                    contentDescription = "프로필 사진"
                    role = Role.Image
                    traversalIndex = 0f
                },
        )
        Box(
            modifier = Modifier
                .height(1.dp)
                .fillMaxWidth()
                .clearAndSetSemantics { },
        )
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .selectableGroup()
                .semantics {
                    contentDescription = "크기"
                    traversalIndex = 2f
                },
            horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.Start),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Row(
                modifier = Modifier
                    .weight(1f),
                horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.Start),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Row(
                    modifier = Modifier
                        .weight(1f)
                        .selectable(selected = selectedSize == "00000000-0000-0000-0000-000000000008", role = Role.RadioButton, onClick = { onAction("00000000-0000-0000-0000-00000000003d"); selectedSize = "00000000-0000-0000-0000-000000000008" })
                        .absolutePadding(left = 16.dp, top = 12.dp, right = 16.dp, bottom = 12.dp),
                    horizontalArrangement = Arrangement.Center,
                    verticalAlignment = Alignment.Top,
                ) {
                    BasicText(
                        text = "작게",
                    )
                }
                Box(
                    modifier = Modifier
                        .weight(1f)
                        .selectable(selected = selectedSize == "00000000-0000-0000-0000-00000000000a", role = Role.RadioButton, onClick = { onAction("00000000-0000-0000-0000-00000000003e"); selectedSize = "00000000-0000-0000-0000-00000000000a" })
                        .semantics {
                            contentDescription = "크게"
                        }
                        .absolutePadding(left = 16.dp, top = 12.dp, right = 16.dp, bottom = 12.dp),
                )
            }
        }
        BasicTextField(
            modifier = Modifier
                .height(44.dp)
                .fillMaxWidth()
                .semantics {
                    contentDescription = "메모"
                    traversalIndex = 3f
                }
                .absolutePadding(left = 12.dp, top = 8.dp, right = 12.dp, bottom = 8.dp),
            value = memo,
            onValueChange = { memo = it },
        )
        Box(
            modifier = Modifier
                .height(48.dp)
                .fillMaxWidth()
                .widthIn(min = 120.dp)
                .clickable(enabled = !(isSaving), role = Role.Button, onClick = { onAction("00000000-0000-0000-0000-00000000003c") })
                .semantics {
                    contentDescription = "저장"
                    traversalIndex = 4f
                },
        )
        LazyVerticalGrid(
            modifier = Modifier
                .fillMaxWidth()
                .semantics {
                    traversalIndex = 5f
                },
            columns = GridCells.Fixed(2),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            item {
                BasicText(
                    text = "새 기능",
                )
            }
            item {
                Box(
                    modifier = Modifier
                        .size(24.dp, 24.dp)
                        .clearAndSetSemantics { },
                )
            }
        }
        Dialog(
            onDismissRequest = { onAction("00000000-0000-0000-0000-00000000003f") },
        ) {
            Box(
                modifier = Modifier
                    .size(320.dp, 120.dp)
                    .semantics {
                        paneTitle = "알림"
                        traversalIndex = 6f
                    },
            ) {
                BasicText(
                    modifier = Modifier
                        .offset(x = 16.dp, y = 16.dp),
                    text = "저장되었습니다",
                )
            }
        }
    }
}
