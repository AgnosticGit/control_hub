import { expect, test, type Locator } from "@playwright/test";
import { collectErrors, ingestSpans, login, randomHex } from "./helpers";

interface LayoutNode { id: string; name: string; x: number; y: number }
async function nodes(map: Locator): Promise<LayoutNode[]> {
  return JSON.parse((await map.getAttribute("data-layout-nodes")) ?? "[]") as LayoutNode[];
}
async function send(name: string) {
  const startNs=BigInt(Date.now()-2000)*1_000_000n;
  await ingestSpans(name,[{traceId:randomHex(16),spanId:randomHex(8),name:"persisted map test",kind:2,startNs,endNs:startNs+1_000_000n}]);
}

test("open maps adopt backend revisions together while keeping camera and existing coordinates",async({page})=>{
  test.setTimeout(120000);
  const errors=collectErrors(page);
  const first=`map-saved-${randomHex(4)}`;
  await send(first);
  await login(page,"isolation@karbird.local","/app/isolation/isolation-test");
  const map=page.getByRole("application",{name:/^Service map/});
  const nowNs=BigInt(Date.now())*1_000_000n;
  const from=(nowNs-600_000_000_000n).toString();
  const to=(nowNs+60_000_000_000n).toString();
  await expect.poll(async()=>{
    const response=await page.request.get(`/api/v1/projects/isolation/environments/isolation-test/topology?from=${from}&to=${to}&live=true`);
    if(!response.ok()) return false;
    const topology=await response.json() as {nodes:Array<{name:string}>};
    return topology.nodes.some(n=>n.name===first);
  },{timeout:60000}).toBe(true);
  await expect.poll(async()=>(await nodes(map)).some(n=>n.name===first),{timeout:30000}).toBe(true);
  const initial=(await nodes(map)).find(n=>n.name===first);
  if(!initial) throw new Error("first node is missing");
  const other=await page.context().newPage();
  try {
    await other.goto("/app/isolation/isolation-test");
    const otherMap=other.getByRole("application",{name:/^Service map/});
    await expect.poll(async()=>(await nodes(otherMap)).find(n=>n.id===initial.id)).toEqual(initial);
    await map.focus();
    await map.press("+");
    const camera=await map.getAttribute("data-camera");
    const next=`map-new-${randomHex(4)}`;
    await send(next);
    await expect.poll(async()=>(await nodes(map)).some(n=>n.name===next),{timeout:60000}).toBe(true);
    await expect.poll(async()=>(await nodes(otherMap)).some(n=>n.name===next),{timeout:60000}).toBe(true);
    expect((await nodes(map)).find(n=>n.id===initial.id)).toEqual(initial);
    expect((await nodes(otherMap)).find(n=>n.id===initial.id)).toEqual(initial);
    await expect(map).toHaveAttribute("data-camera",camera ?? "");
    expect(errors).toEqual([]);
  } finally { await other.close(); }
});

test("an open historical map adopts telemetry processed after its snapshot was cached",async({page})=>{
  test.setTimeout(90000);
  await login(page,"isolation@karbird.local","/app/isolation/isolation-test");
  const fromMs=Math.floor(Date.now()/60000)*60000-24*3600000;
  const toMs=fromMs+120000;
  const fromNs=BigInt(fromMs)*1_000_000n;
  const toNs=BigInt(toMs)*1_000_000n;
  const prefix="/api/v1/projects/isolation/environments/isolation-test";
  const cached=await page.request.get(`${prefix}/topology?from=${fromNs}&to=${toNs}`);
  expect(cached.status()).toBe(200);
  const name=`history-late-${randomHex(4)}`;
  const startNs=fromNs+60_000_000_000n;
  await ingestSpans(name,[{traceId:randomHex(16),spanId:randomHex(8),name:"late historical",kind:2,startNs,endNs:startNs+1_000_000n}]);
  await expect.poll(async()=>{
    const response=await page.request.get(`${prefix}/services?from=${fromNs}&to=${toNs}`);
    const result=await response.json() as {services:Array<{name:string}>};
    return result.services.some(s=>s.name===name);
  }).toBe(true);
  await expect.poll(async()=>{
    const response=await page.request.get(`${prefix}/topology?from=${fromNs}&to=${toNs}`);
    const result=await response.json() as {nodes:Array<{name:string}>};
    return result.nodes.some(node=>node.name===name);
  },{timeout:45000}).toBe(true);
  const dates=await page.evaluate(([from,to])=>{
    const input=(ms:number)=>{const d=new Date(ms);const pad=(n:number)=>String(n).padStart(2,"0");return `${d.getFullYear()}-${pad(d.getMonth()+1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;};
    return {from:input(from ?? 0),to:input(to ?? 0)};
  },[fromMs,toMs]);
  const timeline=page.getByRole("region",{name:"Timeline"});
  await timeline.getByRole("button",{name:"Range",exact:true}).click();
  await timeline.getByLabel("From",{exact:true}).fill(dates.from);
  await timeline.getByLabel("To",{exact:true}).fill(dates.to);
  await timeline.getByRole("button",{name:"Load range"}).click();
  await expect(timeline.getByText("historical",{exact:true})).toBeVisible();
  const map=page.getByRole("application",{name:/^Service map/});
  await expect.poll(async()=>(await nodes(map)).some(n=>n.name===name),{timeout:45000}).toBe(true);
});
