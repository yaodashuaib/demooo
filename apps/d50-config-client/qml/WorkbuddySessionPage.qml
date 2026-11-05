import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

GroupBox {
    title: qsTr("WorkBuddy 会话")

    ColumnLayout {
        anchors.fill: parent
        spacing: 8

        Label {
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            text: qsTr("WorkBuddy 余额接口只认网站 Cookie（与 CodexBar 相同，不读本地加密 token）。在浏览器登录 www.workbuddy.cn 后，从开发者工具复制请求的 Cookie，粘贴并保存。")
            opacity: 0.8
        }

        Label {
            text: configVm.workbuddyConfigured
                  ? qsTr("状态：已保存会话")
                  : qsTr("状态：未配置")
            font.bold: true
        }

        ScrollView {
            Layout.fillWidth: true
            Layout.preferredHeight: 88
            TextArea {
                id: cookieField
                width: parent.width
                wrapMode: TextEdit.WrapAnywhere
                placeholderText: qsTr("粘贴 Cookie 头…")
                text: configVm.workbuddyCookieDraft
                onTextChanged: configVm.workbuddyCookieDraft = text
            }
        }

        RowLayout {
            Button {
                text: qsTr("保存会话")
                onClicked: configVm.saveWorkbuddySession()
            }
            Button {
                text: qsTr("清除")
                onClicked: configVm.clearWorkbuddySession()
            }
        }
    }
}
