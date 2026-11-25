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
import { ink } from "../theme";

/** 一句结论（保存成功 / 失败原因）。 */
type Verdict = { tone: "ok" | "bad"; text: string } | null;

/** 保存成功的统一说法：这一页改完都要重启。 */
const SAVED: Verdict = { tone: "ok", text: "已保存，重启隧道后生效" };

/** 写法示例：左列能直接粘进列表，右列说明它代表什么。 */
const EXAMPLES: [string, string][] = [
  ["192.168.0.0/16", "整个局域网段，家用路由最常见"],
  ["10.0.0.0/8", "公司内网、容器网络"],
  ["172.16.0.0/12", "Docker 默认网段的一部分"],
  ["10.1.2.3", "单个地址，不用写 /32"],
  ["224.0.0.0/4", "组播：投屏、mDNS 发现"],
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
          title="网络参数"
          hint={`正在改「${view().profile}」。改完要重启隧道才生效；MTU、超时与最大会话数在「代理配置 → 编辑」里。`}
        />
        <View style={{ flexDirection: "column", gap: 14 }}>
          <Setting
            label="启用 IPv6"
            hint="关掉后只转发 IPv4：IPv6 站点会先等超时再回落，访问会明显变慢。"
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
            label="自动配置系统路由"
            hint="关掉后需要自己写路由规则，否则流量不会进隧道。"
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
            label="卸载模式"
            hint="Linux 上把隧道放进独立的网络命名空间；Windows 上没有作用，保持关闭即可。"
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
          title="UDP 转发"
          hint="让 UDP（DNS、游戏、视频通话）也能走上游代理；服务器留空表示不用这个功能。"
        />
        <Field label="转发服务器" hint="host:port，例如 127.0.0.1:7300">
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
            <Field label="连接数" hint="同时保持几条连接，默认 5">
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
            <Field label="保活间隔（秒）" hint="空闲多久发一次保活包，默认 30">
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
            label="保存"
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
          title="路由绕过"
          hint="每行一个 IP 或 CIDR；这些地址直连，不经过上游代理。tun2proxy 会把整行当地址解析，所以写不进注释。"
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
            label="填入默认值"
            onPress={() => {
              bypass.set(view().defaultBypass);
              bypassVerdict.set(null);
            }}
          />
          <Button
            label="保存"
            variant="primary"
            onPress={() => bypassVerdict.set(save(api.networkSaveBypass(bypass()), reload))}
          />
          {bypassVerdict() ? (
            <Note tone={bypassVerdict()!.tone} text={bypassVerdict()!.text} />
          ) : null}
        </View>
        <Caption>写法示例：左列可以整行粘进上面的列表，右列说明它代表什么。</Caption>
        <View style={{ flexDirection: "column", gap: 4 }}>
          {EXAMPLES.map(([pattern, meaning]) => (
            <View style={{ flexDirection: "row", alignItems: "center", gap: 16 }}>
              <Text style={{ width: 140, fontSize: 10, color: ink.card() }}>{pattern}</Text>
              <Text style={{ flex: 1, fontSize: 10, color: ink.muted() }}>{meaning}</Text>
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
    return SAVED;
  }
  return { tone: "bad", text: result.error ?? "保存失败，看看日志里的原因" };
}
