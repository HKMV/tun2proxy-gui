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
import { ink } from "../theme";

/** 一句结论（保存成功 / 失败原因）。 */
type Verdict = { tone: "ok" | "bad"; text: string } | null;

/** 保存成功的统一说法：这一页改完都要重启。写成函数——语言会变，常量会冻住。 */
const saved = (): Verdict => ({
  tone: "ok",
  text: t("已保存，重启隧道后生效", "Saved — restart the tunnel to apply"),
});

/** 写法示例：左列能直接粘进列表，右列说明它代表什么。 */
const EXAMPLES: [string, string, string][] = [
  ["192.168.0.0/16", "整个局域网段，家用路由最常见", "The whole LAN range — what a home router hands out"],
  ["10.0.0.0/8", "公司内网、容器网络", "Corporate intranets and container networks"],
  ["172.16.0.0/12", "Docker 默认网段的一部分", "Part of Docker's default range"],
  ["10.1.2.3", "单个地址，不用写 /32", "A single address — no need to write /32"],
  ["224.0.0.0/4", "组播：投屏、mDNS 发现", "Multicast: screen casting and mDNS discovery"],
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
          title={t("网络参数", "Network options")}
          hint={t(
            `正在改「${view().profile}」。改完要重启隧道才生效；MTU、超时与最大会话数在「代理配置 → 编辑」里。`,
            `Editing “${view().profile}”. Restart the tunnel to apply; MTU, timeouts and the session limit live under Profiles → Edit.`,
          )}
        />
        <View style={{ flexDirection: "column", gap: 14 }}>
          <Setting
            label={t("启用 IPv6", "Enable IPv6")}
            hint={t(
              "关掉后只转发 IPv4：IPv6 站点会先等超时再回落，访问会明显变慢。",
              "When off only IPv4 is forwarded: IPv6 sites time out first and then fall back, which feels slow.",
            )}
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
            label={t("自动配置系统路由", "Configure system routes")}
            hint={t(
              "关掉后需要自己写路由规则，否则流量不会进隧道。",
              "When off you have to write the routing rules yourself, otherwise traffic never enters the tunnel.",
            )}
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
            label={t("卸载模式", "Unshare mode")}
            hint={t(
              "Linux 上把隧道放进独立的网络命名空间；Windows 上没有作用，保持关闭即可。",
              "On Linux the tunnel gets its own network namespace; on Windows it does nothing, so leave it off.",
            )}
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
          title={t("UDP 转发", "UDP forwarding")}
          hint={t(
            "让 UDP（DNS、游戏、视频通话）也能走上游代理；服务器留空表示不用这个功能。",
            "Lets UDP (DNS, games, video calls) go through the upstream proxy; leave the server empty to disable it.",
          )}
        />
        <Field
          label={t("转发服务器", "Forwarding server")}
          hint={t("host:port，例如 127.0.0.1:7300", "host:port, e.g. 127.0.0.1:7300")}
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
              label={t("连接数", "Connections")}
              hint={t("同时保持几条连接，默认 5", "Connections to keep open; default 5")}
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
              label={t("保活间隔（秒）", "Keep-alive (s)")}
              hint={t("空闲多久发一次保活包，默认 30", "Idle time before a keep-alive; default 30")}
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
            label={t("保存", "Save")}
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
          title={t("路由绕过", "Route bypass")}
          hint={t(
            "每行一个 IP 或 CIDR；这些地址直连，不经过上游代理。tun2proxy 会把整行当地址解析，所以写不进注释。",
            "One IP or CIDR per line — these go direct, never through the upstream proxy. tun2proxy parses the whole line as an address, so comments are not allowed.",
          )}
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
            label={t("填入默认值", "Fill in defaults")}
            onPress={() => {
              bypass.set(view().defaultBypass);
              bypassVerdict.set(null);
            }}
          />
          <Button
            label={t("保存", "Save")}
            variant="primary"
            onPress={() => bypassVerdict.set(save(api.networkSaveBypass(bypass()), reload))}
          />
          {bypassVerdict() ? (
            <Note tone={bypassVerdict()!.tone} text={bypassVerdict()!.text} />
          ) : null}
        </View>
        <Caption>
          {t(
            "写法示例：左列可以整行粘进上面的列表，右列说明它代表什么。",
            "Examples: the left column can be pasted into the list above as-is, the right column says what it covers.",
          )}
        </Caption>
        <View style={{ flexDirection: "column", gap: 4 }}>
          {EXAMPLES.map(([pattern, zh, en]) => (
            <View style={{ flexDirection: "row", alignItems: "center", gap: 16 }}>
              <Text style={{ width: 140, fontSize: 10, color: ink.card() }}>{pattern}</Text>
              <Text style={{ flex: 1, fontSize: 10, color: ink.muted() }}>{t(zh, en)}</Text>
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
  return { tone: "bad", text: result.error ?? t("保存失败，看看日志里的原因", "Save failed — check the log for the reason") };
}
