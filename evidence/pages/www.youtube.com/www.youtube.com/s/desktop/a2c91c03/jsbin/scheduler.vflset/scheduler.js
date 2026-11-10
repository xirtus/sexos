(function(){'use strict';var f,aa=typeof Object.create=="function"?Object.create:function(a){function b(){}
b.prototype=a;return new b},g=typeof Object.defineProperties=="function"?Object.defineProperty:function(a,b,c){if(a==Array.prototype||a==Object.prototype)return a;
a[b]=c.value;return a};
function ba(a){a=["object"==typeof globalThis&&globalThis,a,"object"==typeof window&&window,"object"==typeof self&&self,"object"==typeof global&&global];for(var b=0;b<a.length;++b){var c=a[b];if(c&&c.Math==Math)return c}throw Error("Cannot find global object");}
var ca=ba(this);function h(a,b){if(b)a:{var c=ca;a=a.split(".");for(var d=0;d<a.length-1;d++){var e=a[d];if(!(e in c))break a;c=c[e]}a=a[a.length-1];d=c[a];b=b(d);b!=d&&b!=null&&g(c,a,{configurable:!0,writable:!0,value:b})}}
var l;if(typeof Object.setPrototypeOf=="function")l=Object.setPrototypeOf;else{var m;a:{var da={a:!0},n={};try{n.__proto__=da;m=n.a;break a}catch(a){}m=!1}l=m?function(a,b){a.__proto__=b;if(a.__proto__!==b)throw new TypeError(a+" is not extensible");return a}:null}var p=l;
function ea(a){var b=0;return function(){return b<a.length?{done:!1,value:a[b++]}:{done:!0}}}
function r(){var a=fa,b=typeof Symbol!="undefined"&&Symbol.iterator&&a[Symbol.iterator];if(b)return b.call(a);if(typeof a.length=="number")return{next:ea(a)};throw Error(String(a)+" is not an iterable or ArrayLike");}
function t(a,b){var c;b&&!b.done&&(c=a["return"])&&c.call(a)}
h("Symbol",function(a){function b(k){if(this instanceof b)throw new TypeError("Symbol is not a constructor");return new c(d+(k||"")+"_"+e++,k)}
function c(k,q){this.h=k;g(this,"description",{configurable:!0,writable:!0,value:q})}
if(a)return a;c.prototype.toString=function(){return this.h};
var d="jscomp_symbol_"+(Math.random()*1E9>>>0)+"_",e=0;return b});
h("Symbol.dispose",function(a){return a?a:Symbol("Symbol.dispose")});/*

 Copyright The Closure Library Authors.
 SPDX-License-Identifier: Apache-2.0
*/
var u=this||self;function v(a){a=a.split(".");for(var b=u,c=0;c<a.length;c++)if(b=b[a[c]],b==null)return null;return b}
function w(a,b){a=a.split(".");for(var c=u,d;a.length&&(d=a.shift());)a.length||b===void 0?c[d]&&c[d]!==Object.prototype[d]?c=c[d]:c=c[d]={}:c[d]=b}
;function x(){this.B=this.B;this.D=this.D}
x.prototype.B=!1;x.prototype.dispose=function(){this.B||(this.B=!0,this.I())};
x.prototype[Symbol.dispose]=function(){this.dispose()};
x.prototype.I=function(){if(this.D)for(;this.D.length;)this.D.shift()()};var y=u.window,z,A,B=(y==null?void 0:(z=y.yt)==null?void 0:z.config_)||(y==null?void 0:(A=y.ytcfg)==null?void 0:A.data_)||{};w("yt.config_",B);function C(a){a=D(a);return typeof a==="string"&&a==="false"?!1:!!a}
function D(a,b){var c={};a=("EXPERIMENT_FLAGS"in B?B.EXPERIMENT_FLAGS:c)[a];return a!==void 0?a:b}
;var E=1E3/60,ha=Number(D("web_emulated_idle_callback_delay",300)||0),F=E-3,fa=[8,5,4,3,2,1,0];function G(a){a=a===void 0?{}:a;x.call(this);var b=this;this.j=[];this.i={};this.G=this.h=0;this.F=this.m=!1;this.A=[];this.C=this.H=!1;var c=r(),d=c.next();try{for(;!d.done;d=c.next())this.j[d.value]=[]}finally{t(c,d)}this.l=0;this.S=a.timeout||1;this.v=F;this.o=0;this.J=this.V.bind(this);this.R=this.W.bind(this);this.M=this.T.bind(this);this.N=this.U.bind(this);this.O=this.L.bind(this);this.P=function(){b.L(performance.now())};
this.K=!!window.requestIdleCallback&&!!window.cancelIdleCallback&&!C("disable_scheduler_requestIdleCallback");((this.u=a.useRaf!==!1&&!!window.requestAnimationFrame)||C("enable_scheduler_simulate_raf"))&&document.addEventListener("visibilitychange",this.J)}
G.prototype=aa(x.prototype);G.prototype.constructor=G;if(p)p(G,x);else for(var H in x)if(H!="prototype")if(Object.defineProperties){var I=Object.getOwnPropertyDescriptor(x,H);I&&Object.defineProperty(G,H,I)}else G[H]=x[H];function J(a,b){var c=Date.now();K(b);b=Date.now()-c;a.m||(a.v-=b)}
function L(a,b,c,d){++a.G;if(c===10)return J(a,b),a.G;var e=a.G;a.i[e]=b;a.m&&!d?a.A.push({id:e,priority:c}):(a.j[c].push(e),a.F||a.m||(a.h!==0&&M(a)!==a.o&&N(a),a.start()));return e}
function O(a){a.A.length=0;for(var b=5;b>=0;b--)a.j[b].length=0;a.j[8].length=0;a.i={};N(a)}
function M(a){var b=C("enable_scheduler_simulate_raf");if(a.j[8].length){if(a.C)return 4;if(b){if(!document.hidden)return a.u?3:5}else if(!document.hidden&&a.u)return 3}for(var c=5;c>=a.l;c--)if(a.j[c].length>0)return c>0?b?document.hidden?2:a.u?3:5:!document.hidden&&a.u?3:2:1;return 0}
function P(a){var b=v("yt.logging.errors.log");b&&b(a)}
function K(a){try{a()}catch(b){P(b)}}
function Q(a){var b=r(),c=b.next();try{for(;!c.done;c=b.next())if(a.j[c.value].length)return!0}finally{t(b,c)}return!1}
f=G.prototype;f.U=function(a){var b=void 0;a&&(b=a.timeRemaining());this.H=!0;R(this,b);this.H=!1};
f.W=function(){R(this)};
f.T=function(){S(this)};
f.L=function(a){this.C=!0;var b=M(this);b===4&&b!==this.o&&(N(this),this.start());R(this,void 0,a);this.C=!1};
f.V=function(){document.hidden||S(this);this.h&&(N(this),this.start())};
function S(a){N(a);a.m=!0;for(var b=Date.now(),c=a.j[8];c.length;){var d=c.shift(),e=a.i[d];delete a.i[d];e&&K(e)}T(a);a.m=!1;Q(a)&&a.start();a.v-=Date.now()-b}
function T(a){for(var b=0,c=a.A.length;b<c;b++){var d=a.A[b];a.j[d.priority].push(d.id)}a.A.length=0}
function R(a,b,c){a.C&&a.o===4&&a.h||N(a);a.m=!0;b=Date.now()+(b||a.v);for(var d=a.j[5];d.length;){var e=d.shift(),k=a.i[e];delete a.i[e];if(k)try{k(c)}catch(na){P(na)}}for(d=a.j[4];d.length;)c=d.shift(),e=a.i[c],delete a.i[c],e&&K(e);d=a.H?0:1;d=a.l>d?a.l:d;if(!(Date.now()>=b)){do{a:{e=a;k=d;for(var q=3;q>=k;q--)for(c=e.j[q];c.length;){var Y=c.shift(),Z=e.i[Y];delete e.i[Y];if(Z){c=Z;break a}}c=null}c&&K(c)}while(c&&Date.now()<b)}a.m=!1;T(a);a.v=F;Q(a)&&a.start()}
f.start=function(){this.F=!1;if(this.h===0)switch(this.o=M(this),this.o){case 1:var a=this.N;this.h=this.K?window.requestIdleCallback(a,{timeout:3E3}):window.setTimeout(a,ha);break;case 2:this.h=window.setTimeout(this.R,this.S);break;case 3:this.h=window.requestAnimationFrame(this.O);break;case 4:this.h=window.setTimeout(this.M,0);break;case 5:this.h=window.setTimeout(this.P,E)}};
function N(a){if(a.h){switch(a.o){case 1:var b=a.h;a.K?window.cancelIdleCallback(b):window.clearTimeout(b);break;case 5:case 2:case 4:window.clearTimeout(a.h);break;case 3:window.cancelAnimationFrame(a.h)}a.h=0}}
f.I=function(){O(this);N(this);(this.u||C("enable_scheduler_simulate_raf"))&&document.removeEventListener("visibilitychange",this.J);x.prototype.I.call(this)};var U=v("yt.scheduler.instance.timerIdMap_")||{},ia=Number(D("kevlar_tuner_scheduler_soft_state_timer_ms",800)||0),V=0,W=0;function X(){var a=v("ytglobal.schedulerInstanceInstance_");if(!a||a.B)a=new G(("scheduler"in B?B.scheduler:void 0)||{}),w("ytglobal.schedulerInstanceInstance_",a);return a}
function ja(){ka();var a=v("ytglobal.schedulerInstanceInstance_");a&&(a&&typeof a.dispose=="function"&&a.dispose(),w("ytglobal.schedulerInstanceInstance_",null))}
function ka(){O(X());for(var a in U)U.hasOwnProperty(a)&&delete U[Number(a)]}
function la(a,b,c){if(!c)return c=c===void 0,-L(X(),a,b,c);var d=window.setTimeout(function(){var e=L(X(),a,b);U[d]=e},c);
return d}
function ma(a){var b=X();J(b,a)}
function oa(a){var b=X();if(a<0)delete b.i[-a];else{var c=U[a];c?(delete b.i[c],delete U[a]):window.clearTimeout(a)}}
function pa(){qa()}
function qa(){window.clearTimeout(V);X().start()}
function ra(){var a=X();N(a);a.F=!0;window.clearTimeout(V);V=window.setTimeout(pa,ia)}
function sa(){window.clearTimeout(W);W=window.setTimeout(function(){ta(0)},ia)}
function ta(a){sa();var b=X();b.l=a;b.start()}
function ua(a){sa();var b=X();b.l>a&&(b.l=a,b.start())}
function va(){window.clearTimeout(W);var a=X();a.l=0;a.start()}
;v("yt.scheduler.initialized")||(w("yt.scheduler.instance.dispose",ja),w("yt.scheduler.instance.addJob",la),w("yt.scheduler.instance.addImmediateJob",ma),w("yt.scheduler.instance.cancelJob",oa),w("yt.scheduler.instance.cancelAllJobs",ka),w("yt.scheduler.instance.start",qa),w("yt.scheduler.instance.pause",ra),w("yt.scheduler.instance.setPriorityThreshold",ta),w("yt.scheduler.instance.enablePriorityThreshold",ua),w("yt.scheduler.instance.clearPriorityThreshold",va),w("yt.scheduler.initialized",!0));}).call(this);
