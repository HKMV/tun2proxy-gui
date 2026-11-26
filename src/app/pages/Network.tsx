//! 网络与绕过页：当前位置配置的网卡参数、UDP 转发与绕过列表。
//!
//! 这一页改的都是**当前那套配置**，改完要重启隧道才生效——每块都写着这句话。
//!
//! # 为什么底部的「写法示例」是一张表
//!
//! tun2proxy 把绕过清单的**整行**当地址解析，所以示例里不能带 `#` 注释：用户
//! 照着注释粘进去，那一行就变成解析不出来的地址。示例单独摆成「能直接粘的
//! 那一列 + 说明那一列」，两列分开。

import { signal, Text, View } from "nsh";
import type { JSX } from "nsh";

import * as api from "../api";
import { Button, Card, CardHead, Caption, Field, Note, Setting, TextBox, Toggle } from "../components";
import { t } from "../i18n";
import type { MessageKey } from "../i18n";
import { ink } from "../theme";

/** 一句结论（保存成功 / 失败原因）。 */
type Verdict = { tone: "ok" | "bad"; text: string } | null;

/** 保存成功的统一说法：这一页改完都要重启。写成函数——语言会变，常量会冻住。 */
const saved = (): Verdict => ({
  tone: "ok",
  text: t("common.savedRestart"),
});

/** 写法示例：左列能直接粘进列表，右列（key）说明它代表什么。 */
const EXAMPLES: [string, MessageKey][] = [
  ["192.168.0.0/16", "network.example.lan"],
  ["10.0.0.0/8", "network.example.corp"],
  ["172.16.0.0/12", "network.example.docker"],
  ["10.1.2.3", "network.example.single"],
  ["224.0.0.0/4", "network.example.multicast"],
];

export function NetworkPage(_props: { snap: () => api.Snapshot }): JSX.Element {
  const view = signal<api.NetworkView>(api.networkView());

  // 三段草稿：UDP 转发的三项与绕过清单，点保存才进配置。
  const server = signal(view().udpgwServer);
  const connections = signal(view().udpgwConnections);
  const keepalive = signal(view().udpgwKeepalive);
  const udpVerdict = signal<Verdict>(null);
  const bypass = signal(view().bypass);
  const bypassVerdict = signal<Verdict>(null);

  const reload = () => view.set(api.networkView());

  return (
    <View style={{ flexDirection: "column", gap: 16 }}>
      {/* 网络参数 */}
      <Card>
        <CardHead
          title={t("network.options")}
          hint={t("network.editingProfile", { profile: view().profile })}
        />
        <View style={{ flexDirection: "column", gap: 14 }}>
          <Setting
            label={t("network.ipv6")}
            hint={t("network.ipv6Hint")}
          >
            <Toggle
              on={view().ipv6}
              onToggle={() => {
                api.networkSetFlag("ipv6", !view().ipv6);
                reload();
              }}
            />
          </Setting>
          <Setting
            label={t("network.autoRoute")}
            hint={t("network.autoRouteHint")}
          >
            <Toggle
              on={view().setup}
              onToggle={() => {
                api.networkSetFlag("setup", !view().setup);
                reload();
              }}
            />
          </Setting>
          <Setting
            label={t("network.unshare")}
            hint={t("network.unshareHint")}
          >
            <Toggle
              on={view().unshare}
              onToggle={() => {
                api.networkSetFlag("unshare", !view().unshare);
                reload();
              }}
            />
          </Setting>
        </View>
      </Card>

      {/* UDP 转发 */}
      <Card>
        <CardHead
          title={t("network.udp")}
          hint={t("network.udpHint")}
        />
        <Field
          label={t("network.server")}
          hint={t("network.serverHint")}
        >
          <TextBox
            value={server()}
            placeholder="127.0.0.1:7300"
            onChange={(value) => {
              server.set(value);
              udpVerdict.set(null);
            }}
          />
        </Field>
        <View style={{ flexDirection: "row", gap: 12 }}>
          <View style={{ flex: 1 }}>
            <Field
              label={t("network.connections")}
              hint={t("network.connectionsHint")}
            >
              <TextBox
                value={connections()}
                placeholder="5"
                onChange={(value) => {
                  connections.set(value);
                  udpVerdict.set(null);
                }}
              />
            </Field>
          </View>
          <View style={{ flex: 1 }}>
            <Field
              label={t("network.keepalive")}
              hint={t("network.keepaliveHint")}
            >
              <TextBox
                value={keepalive()}
                placeholder="30"
                onChange={(value) => {
                  keepalive.set(value);
                  udpVerdict.set(null);
                }}
              />
            </Field>
          </View>
        </View>
        <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
          <Button
            label={t("common.save")}
            variant="primary"
            onPress={() => {
              const result = api.networkSaveUdpgw(server(), connections(), keepalive());
              udpVerdict.set(save(result, reload));
            }}
          />
          {udpVerdict() ? <Note tone={udpVerdict()!.tone} text={udpVerdict()!.text} /> : null}
        </View>
      </Card>

      {/* 路由绕过 */}
      <Card>
        <CardHead
          title={t("network.bypassTitle")}
          hint={t("network.bypassHint")}
        />
        <TextBox
          value={bypass()}
          placeholder="10.0.0.0/8"
          multiline
          height={148}
          onChange={(value) => {
            bypass.set(value);
            bypassVerdict.set(null);
          }}
        />
        <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
          <Button
            label={t("network.fillDefaults")}
            onPress={() => {
              bypass.set(view().defaultBypass);
              bypassVerdict.set(null);
            }}
          />
          <Button
            label={t("common.save")}
            variant="primary"
            onPress={() => bypassVerdict.set(save(api.networkSaveBypass(bypass()), reload))}
          />
          {bypassVerdict() ? (
            <Note tone={bypassVerdict()!.tone} text={bypassVerdict()!.text} />
          ) : null}
        </View>
        <Caption>{t("network.examplesCaption")}</Caption>
        <View style={{ flexDirection: "column", gap: 4 }}>
          {EXAMPLES.map(([pattern, key]) => (
            <View style={{ flexDirection: "row", alignItems: "center", gap: 16 }}>
              <Text style={{ width: 140, fontSize: 10, color: ink.card() }}>{pattern}</Text>
              <Text style={{ flex: 1, fontSize: 10, color: ink.muted() }}>{t(key)}</Text>
            </View>
          ))}
        </View>
      </Card>
    </View>
  );
}

/** 动作结论 → 一句提示；成功时顺手让页面重读一次配置。 */
function save(result: api.Outcome, reload: () => void): Verdict {
  if (result.ok) {
    reload();
    return saved();
  }
  return { tone: "bad", text: result.error ?? t("common.saveFailedLog") };
}
