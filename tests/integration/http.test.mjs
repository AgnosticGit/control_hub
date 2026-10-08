import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { once } from 'node:events';
import { postOversized } from './http.mjs';

test('oversized probe waits for header rejection instead of racing a large body write',async()=>{
  let received=0;
  const reject=(req,res)=>{
    req.on('data',chunk=>{received+=chunk.length;});
    setTimeout(()=>{res.writeHead(413);res.end();},700);
  };
  const server=createServer(reject);
  server.on('checkContinue',reject);
  server.listen(0,'127.0.0.1');
  await once(server,'listening');
  try {
    const status=await postOversized(`http://127.0.0.1:${server.address().port}/body`,{},5*1024*1024);
    assert.equal(status,413);
    assert.equal(received,0,'must not send a rejected body before 100 Continue');
  } finally {server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}
});
