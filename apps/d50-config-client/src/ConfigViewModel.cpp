#include "ConfigViewModel.h"

#include <QJsonArray>
#include <QJsonObject>
#include <QVariantMap>

ConfigViewModel::ConfigViewModel(QObject *parent)
    : QObject(parent)
    , m_allowlist({
          QStringLiteral("claude_code"),
          QStringLiteral("kimi"),
          QStringLiteral("chatgpt"),
          QStringLiteral("workbuddy"),
      })
    , m_bindId(QStringLiteral("chatgpt"))
{
}

void ConfigViewModel::refresh()
{
    const QJsonObject response = m_client.call(QJsonObject{{QStringLiteral("method"), QStringLiteral("config_status")}});
    if (response.value(QStringLiteral("result")).toString() == QStringLiteral("error")) {
        m_error = response.value(QStringLiteral("message")).toString();
        m_online = false;
        emit statusChanged();
        return;
    }
    applyConfigJson(response.value(QStringLiteral("config")).toObject());
}

void ConfigViewModel::bindAgent()
{
    QJsonObject req{
        {QStringLiteral("method"), QStringLiteral("config_bind_agent")},
        {QStringLiteral("id"), m_bindId},
    };
    if (!m_bindName.trimmed().isEmpty()) {
        req.insert(QStringLiteral("name"), m_bindName.trimmed());
    }
    const QJsonObject response = m_client.call(req);
    if (response.value(QStringLiteral("result")).toString() == QStringLiteral("error")) {
        m_error = response.value(QStringLiteral("message")).toString();
        emit statusChanged();
        return;
    }
    applyConfigJson(response.value(QStringLiteral("config")).toObject());
}

void ConfigViewModel::unbindAgent(const QString &id)
{
    const QJsonObject response = m_client.call(QJsonObject{
        {QStringLiteral("method"), QStringLiteral("config_unbind_agent")},
        {QStringLiteral("id"), id},
    });
    if (response.value(QStringLiteral("result")).toString() == QStringLiteral("error")) {
        m_error = response.value(QStringLiteral("message")).toString();
        emit statusChanged();
        return;
    }
    applyConfigJson(response.value(QStringLiteral("config")).toObject());
}

void ConfigViewModel::setBalanceInterval()
{
    const QJsonObject response = m_client.call(QJsonObject{
        {QStringLiteral("method"), QStringLiteral("config_set_balance_interval")},
        {QStringLiteral("secs"), m_balanceRefreshSecs},
    });
    if (response.value(QStringLiteral("result")).toString() == QStringLiteral("error")) {
        m_error = response.value(QStringLiteral("message")).toString();
        emit statusChanged();
        return;
    }
    applyConfigJson(response.value(QStringLiteral("config")).toObject());
}

void ConfigViewModel::applyConfigJson(const QJsonObject &config)
{
    const QJsonObject device = config.value(QStringLiteral("device")).toObject();
    m_online = device.value(QStringLiteral("online")).toBool();
    m_serial = device.value(QStringLiteral("serial")).toString();
    m_error.clear();
    const QJsonArray errors = config.value(QStringLiteral("errors")).toArray();
    if (!errors.isEmpty()) {
        m_error = errors.at(0).toString();
    }

    QVariantList agents;
    const QJsonArray arr = config.value(QStringLiteral("agents")).toArray();
    for (const QJsonValue &value : arr) {
        const QJsonObject obj = value.toObject();
        QVariantMap row;
        row.insert(QStringLiteral("id"), obj.value(QStringLiteral("id")).toString());
        row.insert(QStringLiteral("name"), obj.value(QStringLiteral("name")).toString());
        row.insert(QStringLiteral("agentIndex"), obj.value(QStringLiteral("agent_index")).toInt());
        row.insert(QStringLiteral("balanceOk"), obj.value(QStringLiteral("balance_ok")).toBool());
        row.insert(QStringLiteral("balanceSummary"), obj.value(QStringLiteral("balance_summary")).toString());
        agents.push_back(row);
    }
    m_agents = agents;
    m_balanceRefreshSecs = config.value(QStringLiteral("balance_refresh_secs")).toInt(180);

    const QJsonObject soundLight = config.value(QStringLiteral("sound_light")).toObject();
    const QJsonObject sound = soundLight.value(QStringLiteral("Sound")).toObject();
    const QJsonObject light = soundLight.value(QStringLiteral("Light")).toObject();
    m_soundVolume = sound.value(QStringLiteral("Volume")).toInt(80);
    m_soundEnabled = sound.value(QStringLiteral("AgentPrompt")).toObject().value(QStringLiteral("Enabled")).toBool(true);
    m_lightBrightness = light.value(QStringLiteral("Brightness")).toInt(80);
    const QJsonObject lightPrompt = light.value(QStringLiteral("AgentPrompt")).toObject();
    m_lightEnabled = lightPrompt.value(QStringLiteral("Enabled")).toBool(true);
    m_lightDurationMs = lightPrompt.value(QStringLiteral("DurationMs")).toInt(6000);
    const QJsonObject color = lightPrompt.value(QStringLiteral("Color")).toObject();
    m_lightR = color.value(QStringLiteral("R")).toInt(255);
    m_lightG = color.value(QStringLiteral("G")).toInt(128);
    m_lightB = color.value(QStringLiteral("B")).toInt(0);
    m_workbuddyConfigured = config.value(QStringLiteral("workbuddy_session_configured")).toBool();

    emit statusChanged();
    emit soundLightChanged();
}

void ConfigViewModel::saveWorkbuddySession()
{
    const QJsonObject response = m_client.call(QJsonObject{
        {QStringLiteral("method"), QStringLiteral("config_set_workbuddy_session")},
        {QStringLiteral("cookie"), m_workbuddyCookieDraft},
    });
    if (response.value(QStringLiteral("result")).toString() == QStringLiteral("error")) {
        m_error = response.value(QStringLiteral("message")).toString();
        emit statusChanged();
        return;
    }
    m_workbuddyCookieDraft.clear();
    emit workbuddyDraftChanged();
    refresh();
}

void ConfigViewModel::clearWorkbuddySession()
{
    m_workbuddyCookieDraft.clear();
    emit workbuddyDraftChanged();
    const QJsonObject response = m_client.call(QJsonObject{
        {QStringLiteral("method"), QStringLiteral("config_set_workbuddy_session")},
        {QStringLiteral("cookie"), QString()},
    });
    if (response.value(QStringLiteral("result")).toString() == QStringLiteral("error")) {
        m_error = response.value(QStringLiteral("message")).toString();
        emit statusChanged();
        return;
    }
    refresh();
}

void ConfigViewModel::applySoundLight(bool preview)
{
    QJsonObject soundPrompt{{QStringLiteral("Enabled"), m_soundEnabled}};
    QJsonObject lightPrompt{
        {QStringLiteral("Enabled"), m_lightEnabled},
        {QStringLiteral("Color"),
         QJsonObject{
             {QStringLiteral("R"), m_lightR},
             {QStringLiteral("G"), m_lightG},
             {QStringLiteral("B"), m_lightB},
         }},
        {QStringLiteral("DurationMs"), m_lightDurationMs},
    };
    QJsonObject config{
        {QStringLiteral("Sound"),
         QJsonObject{
             {QStringLiteral("Volume"), m_soundVolume},
             {QStringLiteral("AgentPrompt"), soundPrompt},
         }},
        {QStringLiteral("Light"),
         QJsonObject{
             {QStringLiteral("Brightness"), m_lightBrightness},
             {QStringLiteral("AgentPrompt"), lightPrompt},
         }},
    };
    const QJsonObject response = m_client.call(QJsonObject{
        {QStringLiteral("method"), QStringLiteral("config_set_sound_light")},
        {QStringLiteral("config"), config},
        {QStringLiteral("preview"), preview},
    });
    if (response.value(QStringLiteral("result")).toString() == QStringLiteral("error")) {
        m_error = response.value(QStringLiteral("message")).toString();
        emit statusChanged();
        return;
    }
    if (!preview) {
        refresh();
    }
}
