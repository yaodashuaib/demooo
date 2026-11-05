import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

ApplicationWindow {
    id: root
    width: 820
    height: 720
    visible: true
    title: qsTr("D50 配置客户端")

    Component.onCompleted: configVm.refresh()

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 16
        spacing: 12

        RowLayout {
            Layout.fillWidth: true
            Label {
                text: configVm.online ? qsTr("D50 在线") : qsTr("D50 离线")
                font.pixelSize: 18
                font.bold: true
            }
            Label {
                text: configVm.serial
                opacity: 0.7
            }
            Item { Layout.fillWidth: true }
            Button {
                text: qsTr("刷新")
                onClicked: configVm.refresh()
            }
        }

        Label {
            visible: configVm.error.length > 0
            text: configVm.error
            color: "#b00020"
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        GroupBox {
            title: qsTr("添加 Agent")
            Layout.fillWidth: true

            ColumnLayout {
                anchors.fill: parent
                spacing: 8

                RowLayout {
                    Layout.fillWidth: true
                    Label { text: qsTr("id") }
                    ComboBox {
                        id: bindIdBox
                        Layout.preferredWidth: 180
                        model: configVm.allowlist
                        Component.onCompleted: {
                            const i = configVm.allowlist.indexOf(configVm.bindId)
                            if (i >= 0)
                                currentIndex = i
                        }
                        onActivated: configVm.bindId = currentText
                    }
                    Label { text: qsTr("显示名") }
                    TextField {
                        Layout.fillWidth: true
                        placeholderText: qsTr("可选，默认内置名")
                        text: configVm.bindName
                        onTextChanged: configVm.bindName = text
                    }
                    Button {
                        text: qsTr("绑定")
                        onClicked: configVm.bindAgent()
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    Label { text: qsTr("余额刷新间隔（秒）") }
                    SpinBox {
                        from: 30
                        to: 3600
                        stepSize: 30
                        value: configVm.balanceRefreshSecs
                        onValueModified: configVm.balanceRefreshSecs = value
                    }
                    Button {
                        text: qsTr("应用间隔")
                        onClicked: configVm.setBalanceInterval()
                    }
                    Item { Layout.fillWidth: true }
                }
            }
        }

        GroupBox {
            title: qsTr("已绑定 Agent")
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.minimumHeight: 200
            Layout.preferredHeight: 240

            ListView {
                id: agentList
                anchors.fill: parent
                anchors.margins: 4
                clip: true
                spacing: 6
                model: configVm.agents
                ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

                delegate: Frame {
                    width: agentList.width
                    padding: 8

                    ColumnLayout {
                        anchors.fill: parent
                        spacing: 4
                        RowLayout {
                            Layout.fillWidth: true
                            Label {
                                text: modelData.name + "  [" + modelData.id + "]  #" + modelData.agentIndex
                                font.bold: true
                                Layout.fillWidth: true
                            }
                            Label {
                                text: modelData.balanceOk ? qsTr("余额 OK") : qsTr("暂无数据")
                                color: modelData.balanceOk ? "#2e7d32" : "#b00020"
                            }
                            Button {
                                text: qsTr("解绑")
                                onClicked: configVm.unbindAgent(modelData.id)
                            }
                        }
                        Label {
                            text: modelData.balanceSummary.length > 0
                                  ? modelData.balanceSummary
                                  : qsTr("（无摘要）")
                            opacity: 0.8
                            wrapMode: Text.WordWrap
                            Layout.fillWidth: true
                        }
                    }
                }
            }
        }

        WorkbuddySessionPage {
            Layout.fillWidth: true
            Layout.preferredHeight: 180
            Layout.maximumHeight: 220
        }

        SoundLightPage {
            Layout.fillWidth: true
            Layout.preferredHeight: 160
        }
    }
}
