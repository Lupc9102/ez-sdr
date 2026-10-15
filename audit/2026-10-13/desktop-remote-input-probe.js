'use strict';
const fs = require('fs'), vm = require('vm'), assert = require('assert');
const elements = new Map();
const document = {getElementById(id){if(!elements.has(id))elements.set(id,{value:'',textContent:'',style:{},classList:{toggle(){}},appendChild(){}});return elements.get(id);},createElement(){return {};}};
let socket;
class Socket {constructor(){socket=this;this.readyState=1;}send(){}}
const context = {document,location:{host:'127.0.0.1:5259'},WebSocket:Socket,setTimeout(){}};
vm.createContext(context);
const html = fs.readFileSync('ez-gui/src/web_remote.html','utf8');
vm.runInContext(html.match(/<script>([\s\S]*?)<\/script>/)[1],context);
const input = document.getElementById('freqInput');
input.value = '118700000'; document.activeElement=input;
socket.onmessage({data:JSON.stringify({frequency_hz:100000000,gain_db:10,demod_mode:'NFM',aircraft_count:0,snr_db:20,squelch_db:-60,volume:.8,scanner_active:false,upcoming_passes:[]})});
assert.equal(input.value,100000000);
console.log(JSON.stringify({source:'ez-gui/src/web_remote.html',focused_input_before:'118700000',focused_input_after:input.value,typed_frequency_overwritten:true},null,2));
