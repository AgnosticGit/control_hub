import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { copyFileSync, unlinkSync } from 'node:fs';
import { join } from 'node:path';
import { createServer } from 'node:net';
import { once } from 'node:events';
import { loadEnvFile, root } from '../../scripts/lib.mjs';
import { Session, ISO, DEMO, API, encodeTraces, encodeMetrics, otlpHttp, nowNs, hex, eventually } from './lib.mjs';

async function sendService(name, at = nowNs() - 2_000_000_000n) {
  const result = await otlpHttp('traces', encodeTraces(name, [{traceId:hex(16),spanId:hex(8),name:'saved graph test',kind:2,startNs:at,endNs:at+1_000_000n}]));
  assert.equal(result.status,200);
}

test('concurrent cold reads publish one shared document without resetting revisions',async()=>{
  const session=new Session();
  await session.login('isolation@karbird.local');
  const at=nowNs()-234_000_000_000n;
  const path=`${ISO}/topology?from=${at}&to=${at+1_000_000_000n}`;
  const results=await Promise.all(Array.from({length:24},()=>session.json(path)));
  for(const result of results) {
    assert.equal(result.status,200);
    assert.equal(result.body.updated_at_ns,results[0].body.updated_at_ns);
    assert.deepEqual(result.body.geometry,results[0].body.geometry);
  }
});

test('saved live graph is shared, refreshed on backend changes and conditionally authorized', async () => {
  const session=new Session();
  await session.login('isolation@karbird.local');
  const firstName=`saved-first-${hex(4)}`;
  await sendService(firstName);
  const path=`${ISO}/topology?live=true`;
  let initial;
  await eventually(async()=>{
    const result=await session.json(path);
    assert.equal(result.status,200);
    initial=result.body;
    assert.ok(initial.nodes.some(n=>n.name===firstName));
  },{timeoutMs:60000});
  const firstNode=initial.geometry.plain.nodes.find(n=>n.name===firstName);
  assert.ok(firstNode);
  const concurrent=await Promise.all(Array.from({length:8},()=>session.json(path)));
  for(const result of concurrent) {
    assert.equal(result.status,200);
    assert.deepEqual(result.body.geometry.plain.nodes.find(n=>n.id===firstNode.id),firstNode);
  }
  const full=await session.request(path);
  await full.arrayBuffer();
  assert.equal(full.status,200);
  const etag=full.headers.get('etag');
  assert.ok(etag);
  const conditional=await session.request(path,{headers:{'if-none-match':etag}});
  assert.equal(conditional.status,304);
  assert.equal(await conditional.text(),'');
  const unauthenticated=await fetch(`${API}${path}`,{headers:{'if-none-match':etag}});
  await unauthenticated.arrayBuffer();
  assert.equal(unauthenticated.status,401);
  const foreign=await session.request(`${DEMO}/topology?live=true`,{headers:{'if-none-match':etag}});
  await foreign.arrayBuffer();
  assert.equal(foreign.status,404,'foreign project existence remains hidden even with an ETag');

  const secondName=`saved-second-${hex(4)}`;
  await sendService(secondName);
  let updated;
  await eventually(async()=>{
    const result=await session.json(path);
    assert.equal(result.status,200);
    updated=result.body;
    assert.ok(updated.nodes.some(n=>n.name===secondName));
    assert.ok(updated.geometry.revision>initial.geometry.revision);
  },{timeoutMs:60000});
  const stable=updated.geometry.plain.nodes.find(n=>n.id===firstNode.id);
  assert.deepEqual([stable.x,stable.y],[firstNode.x,firstNode.y]);
  const reopened=new Session();
  await reopened.login('isolation@karbird.local');
  const result=await reopened.json(path);
  assert.equal(result.status,200);
  assert.deepEqual(result.body.geometry,updated.geometry);
});

test('saved historical graph keeps exact range membership',async()=>{
  const session=new Session();
  await session.login('isolation@karbird.local');
  const at=nowNs()-120_000_000_000n;
  const inside=`history-inside-${hex(4)}`;
  const outside=`history-outside-${hex(4)}`;
  await sendService(inside,at);
  await sendService(outside,at+60_000_000_000n);
  const path=`${ISO}/topology?from=${at-1_000_000_000n}&to=${at+2_000_000_000n}`;
  let saved;
  await eventually(async()=>{
    const result=await session.json(path);
    assert.equal(result.status,200);
    saved=result.body;
    assert.ok(saved.nodes.some(n=>n.name===inside));
    assert.ok(!saved.nodes.some(n=>n.name===outside));
  },{timeoutMs:60000});
  const next=await session.json(path);
  assert.deepEqual(next.body.geometry,saved.geometry);
});

test('selected-time placement is saved separately from metrics and follows the machines observed then',async()=>{
  const session=new Session();await session.login('isolation@karbird.local');
  const run=hex(4),service=`placement-${run}`,oldHost=`old-${run}`,newHost=`new-${run}`;
  const earlier=nowNs()-600_000_000_000n,later=nowNs()-5_000_000_000n;
  const metrics=(host,timeNs,cpu)=>encodeMetrics([{attributes:{'host.id':host,'host.name':host},metrics:[{name:'system.cpu.utilization',unit:'1',kind:'gauge',points:[{value:1-cpu,timeNs,attributes:{state:'idle',cpu:'cpu0'}}]},{name:'system.memory.limit',unit:'By',kind:'gauge',points:[{value:1048576,timeNs}]}]}]);
  for(const [host,at,instance] of [[oldHost,earlier,`old-instance-${run}`],[newHost,later,`new-instance-${run}`]]) {
    assert.equal((await otlpHttp('metrics',metrics(host,at,0.2))).status,200);
    assert.equal((await otlpHttp('traces',encodeTraces(service,[{traceId:hex(16),spanId:hex(8),name:'placement move',kind:2,startNs:at,endNs:at+1_000_000n}],{'host.id':host,'host.name':host,'service.instance.id':instance}))).status,200);
  }
  const range=`from=${earlier-1_000_000_000n}&to=${later+2_000_000_000n}`;
  const path=`${ISO}/topology?${range}&at=${earlier+1_000_000_000n}`;
  let before;
  await eventually(async()=>{
    const result=await session.json(path);assert.equal(result.status,200);before=result.body;
    assert.ok(before.geometry.infrastructure.infra.frames.some(f=>f.host_id===oldHost));
    assert.ok(!before.geometry.infrastructure.infra.frames.some(f=>f.host_id===newHost));
    assert.ok(before.nodes.some(n=>n.name===service));
  },{timeoutMs:60000});
  const after=await session.json(`${ISO}/topology?${range}&at=${later+1_000_000_000n}`);
  assert.equal(after.status,200);
  assert.ok(after.body.geometry.infrastructure.infra.frames.some(f=>f.host_id===newHost));
  const full=await session.request(path);const etag=full.headers.get('etag');await full.arrayBuffer();
  assert.equal((await otlpHttp('metrics',metrics(oldHost,earlier+100n,0.8))).status,200);
  await eventually(async()=>{
    const result=await session.json(path,{headers:{'if-none-match':etag}});
    assert.equal(result.status,200,'point metric changes must invalidate the full response ETag');
    const host=result.body.infrastructure_snapshot.hosts.find(h=>h.id===oldHost);
    assert.ok(host.cpu_utilization>=0.79);
    assert.deepEqual(result.body.geometry,before.geometry,'metric-only changes reuse the placement');
  },{timeoutMs:60000});
  const infra=await session.json(`${ISO}/infrastructure?${range}&at=${later+1_000_000_000n}`);
  assert.equal(infra.status,200);
  assert.ok(infra.body.topology.geometry.infrastructure.infra.frames.some(f=>f.host_id===newHost));
});

test('a new API process reads the persisted snapshot after its predecessor stops',async()=>{
  const session=new Session();
  await session.login('isolation@karbird.local');
  const at=nowNs()-180_000_000_000n;
  const path=`${ISO}/topology?from=${at}&to=${at+1_000_000_000n}`;
  const initial=await session.request(path);
  assert.equal(initial.status,200);
  const etag=initial.headers.get('etag');
  const initialBody=await initial.json();
  const server=createServer();
  server.listen(0,'127.0.0.1');
  await once(server,'listening');
  const port=server.address().port;
  await new Promise(resolve=>server.close(resolve));
  const binary=join(root,'.karbird','bin',`map-persistence-test-${hex(4)}${process.platform==='win32'?'.exe':''}`);
  copyFileSync(join(root,'target','debug',`karbird-api${process.platform==='win32'?'.exe':''}`),binary);
  let child;
  const stop=async()=>{
    if(child && child.exitCode===null) {const stopped=once(child,'exit');child.kill();await stopped;}
    child=null;
  };
  const start=async()=>{
    child=spawn(binary,['serve','--addr',`127.0.0.1:${port}`],{cwd:root,stdio:'ignore',env:{...process.env,...loadEnvFile('.karbird/services.env'),...loadEnvFile('.karbird/settings.env')}});
    await eventually(async()=>{
      const response=await fetch(`http://127.0.0.1:${port}/api/v1/health`);
      await response.text();
      assert.equal(response.status,200);
    });
  };
  try {
    for(let iteration=0;iteration<2;iteration++) {
      await start();
      const response=await fetch(`http://127.0.0.1:${port}${path}`,{headers:{cookie:session.cookieHeader(),'if-none-match':etag}});
      assert.equal(response.status,304,'saved document survives process lifetime');
      await response.arrayBuffer();
      const actual=await fetch(`http://127.0.0.1:${port}${path}`,{headers:{cookie:session.cookieHeader()}});
      assert.deepEqual((await actual.json()).geometry,initialBody.geometry);
      await stop();
    }
  } finally {await stop();unlinkSync(binary);}
});
