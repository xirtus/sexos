globalThis.process??={};globalThis.process.env??={};var se=Object.defineProperty,re=(r,e,t)=>e in r?se(r,e,{enumerable:!0,configurable:!0,writable:!0,value:t}):r[e]=t,M=(r,e,t)=>re(r,typeof e!="symbol"?e+"":e,t);const ie="0.0.42",ae=`<svg width="32" height="10" viewBox="0 0 412 186" xmlns="http://www.w3.org/2000/svg" aria-label="Cloudflare" role="img">
  <path fill="#f38020" d="m280.8395,183.31456c11,-26 -4,-38 -19,-38l-148,-2c-4,0 -4,-6 1,-7l150,-2c17,-1 37,-15 43,-33c0,0 10,-21 9,-24a97,97 0 0 0 -187,-11c-38,-25 -78,9 -69,46c-48,3 -65,46 -60,72c0,1 1,2 3,2l274,0c1,0 3,-1 3,-3z"/>
  <path fill="#faae40" d="m330.8395,81.31456c-4,0 -6,-1 -7,1l-5,21c-5,16 3,30 20,31l32,2c4,0 4,6 -1,7l-33,1c-36,4 -46,39 -46,39c0,2 0,3 2,3l113,0l3,-2a81,81 0 0 0 -78,-103"/>
</svg>`,ne="https://workers.cloudflare.com/product/ai-search",P=`Powered by <a href="${ne}" target="_blank" rel="noopener noreferrer">Cloudflare AI Search ${ae}</a>`,A={loadingAriaLabel:"Loading",errorPrefix:"Error:",missingApiUrlError:"The api-url attribute is required. Please provide a valid API URL.",poweredBy:"Powered by",poweredByLinkLabel:"Cloudflare AI Search",placeholder:"Search...",searchButtonLabel:"Search",searchInputAriaLabel:"Search input",searchResultsAriaLabel:"Search results",emptyStateTitle:"Start Searching",emptyStateDescription:"Enter a query to search for results",modalEmptyStateDescription:"Start typing to search",noResultsTitle:"No Results Found",noResultsDescription:'No results found for "{query}"',modalNoResultsTitle:"No results found",modalNoResultsDescription:'No results for "{query}"',resultsCount:"Found {n} result",resultsCountPlural:"Found {n} results",resultsCountOverflow:"Showing {n} of {total} results",modalResultsCount:"{n} result",modalResultsCountPlural:"{n} results",modalResultsCountZero:"0 results",modalResultsCountError:"Error",seeMoreResults:"See more results",groupOther:"Other",favoriteResults:"Favorites",recentResults:"Recent",addFavorite:"Add to favorites",removeFavorite:"Remove from favorites",navigateHint:"Navigate",selectHint:"Select",closeHint:"Close",chatTitle:"Chat",chatPlaceholder:"Type a message...",chatInputAriaLabel:"Chat message input",sendButtonLabel:"Send",sendButtonAriaLabel:"Send message",chatEmptyTitle:"Start a Conversation",chatEmptyDescription:"Send a message to begin chatting",userAvatar:"U",assistantAvatar:"AI",unknownError:"Unknown error",openChatAriaLabel:"Open chat",clearHistoryAriaLabel:"Clear history",minimizeAriaLabel:"Minimize",closeAriaLabel:"Close",historyTitle:"History",newChatButton:"New Chat",clearChatButton:"Clear Chat",toggleSidebarTitle:"Toggle sidebar",deleteChatTitle:"Delete chat",noChatsYet:"No chats yet",yesterday:"Yesterday",justNow:"Just now",minuteAgo:"{n} minute ago",minutesAgo:"{n} minutes ago",hourAgo:"{n} hour ago",hoursAgo:"{n} hours ago",loadingMessages:["Searching...","Digging through results...","Scanning the knowledge base...","Finding the best matches...","Sifting through the data...","Almost there...","Looking far and wide...","Connecting the dots...","Rummaging through pages...","Hunting down answers..."]};function f(r){if(!r||typeof r!="object")return A;const e={...A};for(const t of Object.keys(r)){const s=r[t];if(s!=null){if(t==="loadingMessages"){Array.isArray(s)&&s.length>0&&(e.loadingMessages=s.filter(i=>typeof i=="string"),e.loadingMessages.length===0&&(e.loadingMessages=A.loadingMessages));continue}typeof s=="string"&&(e[t]=s)}}return e}function y(r,e={}){return r.replace(/\{(\w+)\}/g,(t,s)=>Object.hasOwn(e,s)?String(e[s]):t)}function D(r,e){if(!r)return null;try{const t=JSON.parse(r);if(t===null||typeof t!="object"||Array.isArray(t))throw new Error("translations must be a JSON object");return t}catch(t){return console.error(`${e}: invalid translations attribute`,t),null}}const F=`
:host {
  /* Colors - Light Mode */
  --search-snippet-primary-color: #2563eb;
  --search-snippet-primary-hover: #0f51dfff;
  --search-snippet-background: #ffffff;
  --search-snippet-surface: #f8f9fa;
  --search-snippet-text-color: #212529;
  --search-snippet-text-secondary: #6c757d;
  --search-snippet-text-description: #495057;
  --search-snippet-border-color: #dee2e6;
  --search-snippet-hover-background: #f1f3f5;
  --search-snippet-focus-ring: #0066cc40;
  --search-snippet-error-color: #dc3545;
  --search-snippet-error-background: #f8d7da;
  --search-snippet-success-color: #28a745;
  --search-snippet-success-background: #d4edda;
  --search-snippet-warning-color: #ffc107;
  --search-snippet-warning-background: #fff3cd;
  
  /* Message Colors */
  --search-snippet-user-message-bg: #0066cc;
  --search-snippet-user-message-text: #ffffff;
  --search-snippet-assistant-message-bg: #f1f3f5;
  --search-snippet-assistant-message-text: #212529;
  --search-snippet-system-message-bg: #fff3cd;
  --search-snippet-system-message-text: #856404;
  
  /* Typography */
  --search-snippet-font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', 'Roboto', 
                                'Helvetica Neue', Arial, sans-serif, 'Apple Color Emoji', 
                                'Segoe UI Emoji', 'Segoe UI Symbol';
  --search-snippet-font-family-mono: 'SFMono-Regular', Consolas, 'Liberation Mono', Menlo, Courier, monospace;
  --search-snippet-font-size-base: 14px;
  --search-snippet-font-size-sm: 12px;
  --search-snippet-font-size-lg: 16px;
  --search-snippet-font-size-xl: 18px;
  --search-snippet-line-height: 1.5;
  --search-snippet-font-weight-normal: 400;
  --search-snippet-font-weight-medium: 500;
  --search-snippet-font-weight-bold: 600;
  
  /* Spacing */
  --search-snippet-spacing-xs: 4px;
  --search-snippet-spacing-sm: 8px;
  --search-snippet-spacing-md: 12px;
  --search-snippet-spacing-lg: 16px;
  --search-snippet-spacing-xl: 24px;
  --search-snippet-spacing-xxl: 32px;
  
  /* Sizing */
  --search-snippet-width: 100%;
  --search-snippet-max-width: 100%;
  --search-snippet-min-width: 320px;
  --search-snippet-max-height: 600px;
  --search-snippet-input-height: 44px;
  --search-snippet-button-height: 36px;
  --search-snippet-icon-size: 20px;
  
  /* Border */
  --search-snippet-border-width: 1px;
  --search-snippet-border-radius: 18px;
  
  /* Shadows */
  --search-snippet-shadow-sm: 0 1px 2px 0 rgba(0, 0, 0, 0.05);
  --search-snippet-shadow: 0 2px 8px rgba(0, 0, 0, 0.1);
  --search-snippet-shadow-md: 0 4px 12px rgba(0, 0, 0, 0.15);
  --search-snippet-shadow-lg: 0 8px 24px rgba(0, 0, 0, 0.2);
  --search-snippet-shadow-inner: inset 0 2px 4px 0 rgba(0, 0, 0, 0.06);
  
  /* Animation */
  --search-snippet-transition-fast: 150ms ease;
  --search-snippet-transition: 200ms ease;
  --search-snippet-transition-slow: 300ms ease;
  --search-snippet-animation-duration: 0.2s;
  
  /* Z-index */
  --search-snippet-z-dropdown: 1000;
  --search-snippet-z-modal: 1050;
  --search-snippet-z-popover: 1060;
  --search-snippet-z-tooltip: 1070;
  
  /* Layout */
  display: block;
  width: var(--search-snippet-width);
  max-width: var(--search-snippet-max-width);
  min-width: var(--search-snippet-min-width);
  font-family: var(--search-snippet-font-family);
  font-size: var(--search-snippet-font-size-base);
  line-height: var(--search-snippet-line-height);
  color: var(--search-snippet-text-color);


  /* Search */
  --search-snippet-icon-size: 20px;
  --search-snippet-icon-margin-left: 6px;
  --search-snippet-result-icon-size: 20px;
  --search-snippet-result-icon-color: var(--search-snippet-text-secondary);
  --search-snippet-result-icon-active-color: var(--search-snippet-primary-color);
  --search-snippet-result-item-shadow: 0 1px 2px 0 rgba(0, 0, 0, 0.05);

  /* Chat Bubble */
  --chat-bubble-button-shadow: 0 8px 24px rgba(0, 0, 0, 0.2);
  --chat-bubble-window-shadow: 0 8px 24px rgba(0, 0, 0, 0.2);
  --chat-bubble-button-size: 60px;
  --chat-bubble-button-radius: 50%;
  --chat-bubble-button-icon-size: 28px;
  --chat-bubble-button-icon-color: white;
  --chat-bubble-button-bottom: 20px;
  --chat-bubble-button-right: 20px;
  --chat-bubble-button-z-index: 9999;
  --chat-bubble-position: fixed;


}

:host(:not([theme="dark"])) {
  /* Colors - Light Mode */
  --search-snippet-primary-color: #2563eb;
  --search-snippet-primary-hover: #0f51dfff;
  --search-snippet-background: #ffffff;
  --search-snippet-surface: #f8f9fa;
  --search-snippet-text-color: #212529;
  --search-snippet-text-secondary: #6c757d;
  --search-snippet-text-description: #495057;
  --search-snippet-border-color: #dee2e6;
  --search-snippet-hover-background: #f1f3f5;
  --search-snippet-focus-ring: #0066cc40;
  --search-snippet-error-color: #dc3545;
  --search-snippet-error-background: #f8d7da;
  --search-snippet-success-color: #28a745;
  --search-snippet-success-background: #d4edda;
  --search-snippet-warning-color: #ffc107;
  --search-snippet-warning-background: #fff3cd;
  
  /* Message Colors */
  --search-snippet-user-message-bg: #0066cc;
  --search-snippet-user-message-text: #ffffff;
  --search-snippet-assistant-message-bg: #f1f3f5;
  --search-snippet-assistant-message-text: #212529;
  --search-snippet-system-message-bg: #fff3cd;
  --search-snippet-system-message-text: #856404;
}

/* Dark Mode */
@media (prefers-color-scheme: dark) {
  :host(:not([theme="light"])) {
    --search-snippet-primary-color: #2563eb;
    --search-snippet-primary-hover: #0f51dfff;
    --search-snippet-background: #1a1b1e;
    --search-snippet-surface: #25262b;
    --search-snippet-text-color: #c1c2c5;
    --search-snippet-text-secondary: #909296;
    --search-snippet-text-description: #adb5bd;
    --search-snippet-border-color: #373a40;
    --search-snippet-hover-background: #2c2e33;
    --search-snippet-focus-ring: #4dabf740;
    --search-snippet-error-color: #ff6b6b;
    --search-snippet-error-background: #3d1f1f;
    --search-snippet-success-color: #51cf66;
    --search-snippet-success-background: #1f3d24;
    --search-snippet-warning-color: #ffd43b;
    --search-snippet-warning-background: #3d3419;
    
    --search-snippet-user-message-bg: #4dabf7;
    --search-snippet-user-message-text: #1a1b1e;
    --search-snippet-assistant-message-bg: #2c2e33;
    --search-snippet-assistant-message-text: #c1c2c5;
    --search-snippet-system-message-bg: #3d3419;
    --search-snippet-system-message-text: #ffd43b;
    color-scheme: dark;
  }
}

/* Auto theme support */
:host([theme="light"]) {
  color-scheme: light;
}


/* Base reset */
* {
  box-sizing: border-box;
  margin: 0;
  padding: 0;
}

/* Metadata-driven result icons are hidden until an external ::part() rule displays them. */
.result-icon {
  display: none;
  flex: 0 0 var(--search-snippet-result-icon-size);
  width: var(--search-snippet-result-icon-size);
  height: var(--search-snippet-result-icon-size);
  color: var(--search-snippet-result-icon-color);
}

a:hover > .result-icon,
a:focus-visible > .result-icon,
a.active > .result-icon {
  color: var(--search-snippet-result-icon-active-color);
}

/* Container */
.container {
  background: var(--search-snippet-background);
  border: var(--search-snippet-border-width) solid var(--search-snippet-border-color);
  border-radius: var(--search-snippet-border-radius);
  box-shadow: var(--search-snippet-shadow);
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

/* Header */
.header {
  padding: var(--search-snippet-spacing-md);
  border-bottom: var(--search-snippet-border-width) solid var(--search-snippet-border-color);
  background: var(--search-snippet-surface);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--search-snippet-spacing-md);
}

.header-title {
  font-size: var(--search-snippet-font-size-lg);
  font-weight: var(--search-snippet-font-weight-bold);
  color: var(--search-snippet-text-color);
}

/* Input */
.input-wrapper {
  position: relative;
  display: flex;
  align-items: center;
  gap: var(--search-snippet-spacing-sm);
}

.input {
  width: 100%;
  height: var(--search-snippet-input-height);
  padding: var(--search-snippet-spacing-sm) var(--search-snippet-spacing-md);
  font-family: var(--search-snippet-font-family);
  font-size: var(--search-snippet-font-size-base);
  line-height: var(--search-snippet-line-height);
  color: var(--search-snippet-text-color);
  background: var(--search-snippet-background);
  border: var(--search-snippet-border-width) solid var(--search-snippet-border-color);
  border-radius: var(--search-snippet-border-radius);
  outline: none;
  transition: var(--search-snippet-transition);
}

.input:focus {
  border-color: var(--search-snippet-primary-color);
  box-shadow: 0 0 0 3px var(--search-snippet-focus-ring);
}

.input::placeholder {
  color: var(--search-snippet-text-secondary);
}

.input:disabled {
  background: var(--search-snippet-surface);
  cursor: not-allowed;
  opacity: 0.6;
}

/* Button */
.button {
  height: var(--search-snippet-button-height);
  padding: 0 var(--search-snippet-spacing-lg);
  font-family: var(--search-snippet-font-family);
  font-size: var(--search-snippet-font-size-base);
  font-weight: var(--search-snippet-font-weight-medium);
  color: #ffffff;
  background: var(--search-snippet-primary-color);
  border: none;
  border-radius: var(--search-snippet-border-radius);
  cursor: pointer;
  outline: none;
  transition: var(--search-snippet-transition);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: var(--search-snippet-spacing-sm);
  white-space: nowrap;
}

.button:hover:not(:disabled) {
  background: var(--search-snippet-primary-hover);
}

.button:focus-visible {
  box-shadow: 0 0 0 3px var(--search-snippet-focus-ring);
}

.button:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.button-secondary {
  background: var(--search-snippet-surface);
  color: var(--search-snippet-text-color);
  border: var(--search-snippet-border-width) solid var(--search-snippet-border-color);
}

.button-secondary:hover:not(:disabled) {
  background: var(--search-snippet-hover-background);
}

/* Content area */
.content {
  flex: 1;
  overflow-y: auto;
  overflow-x: hidden;
  padding: var(--search-snippet-spacing-md);
}

/* Scrollbar styling */
.content::-webkit-scrollbar {
  width: 8px;
}

.content::-webkit-scrollbar-track {
  background: var(--search-snippet-surface);
}

.content::-webkit-scrollbar-thumb {
  background: var(--search-snippet-border-color);
  border-radius: var(--search-snippet-border-radius);
}

.content::-webkit-scrollbar-thumb:hover {
  background: var(--search-snippet-text-secondary);
}

/* Loading spinner */
.loading {
  display: inline-block;
  width: var(--search-snippet-icon-size);
  height: var(--search-snippet-icon-size);
  border: 2px solid currentColor;
  border-top-color: transparent;
  border-radius: 50%;
  animation: spin 0.6s linear infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

/* Loading message animation */
@keyframes loading-message-in {
  from {
    opacity: 0;
    transform: translateY(8px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

.loading-text {
  font-size: var(--search-snippet-font-size-sm);
  color: var(--search-snippet-text-secondary);
}

.loading-text-animate {
  animation: loading-message-in 0.3s ease-out;
}

/* Error message */
.error {
  padding: var(--search-snippet-spacing-md);
  color: var(--search-snippet-error-color);
  background: var(--search-snippet-error-background);
  border-radius: var(--search-snippet-border-radius);
  font-size: var(--search-snippet-font-size-sm);
}

/* Empty state */
.empty {
  padding: var(--search-snippet-spacing-xl);
  text-align: center;
  color: var(--search-snippet-text-secondary);
}

/* Accessibility */
.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border-width: 0;
}

/* Focus visible polyfill */
.focus-visible:focus {
  outline: 2px solid var(--search-snippet-primary-color);
  outline-offset: 2px;
}

/* Powered by branding - block style (for sidebars) */
.powered-by {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--search-snippet-spacing-xs);
  padding: var(--search-snippet-spacing-sm) var(--search-snippet-spacing-md);
  font-size: var(--search-snippet-font-size-sm);
  color: var(--search-snippet-text-secondary);
  background: var(--search-snippet-surface);
  border-top: var(--search-snippet-border-width) solid var(--search-snippet-border-color);
  margin-top: auto;
  flex-shrink: 0;
}

.powered-by svg {
  width: 16px;
  height: 16px;
  flex-shrink: 0;
}

.powered-by a,
.powered-by-inline a {
  color: var(--search-snippet-text-secondary);
  text-decoration: none;
  transition: color var(--search-snippet-transition-fast);
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.powered-by a:hover,
.powered-by-inline a:hover {
  color: var(--search-snippet-primary-color);
}

/* Powered by branding - inline style (for headers/subtle placement) */
.powered-by-inline {
  font-size: var(--search-snippet-font-size-sm);
  color: var(--search-snippet-text-secondary);
  padding: var(--search-snippet-spacing-xs) 0;
  text-align: center;
}
`,oe="@cf/meta/llama-3.3-70b-instruct-fp8-fast",le=`You rewrite a multi-turn chat into a single standalone search query for a retrieval system.

Inputs: the full conversation in \`messages\`. The final user message is the one to answer; earlier messages are context only.

Rules:
- Output ONLY the rewritten query as plain text. No preamble, no quotes, no markdown, no explanation.
- Resolve pronouns and references (it, that, they, the second one, the previous one, etc.) using prior turns.
- Inline any entities, names, versions, products, or constraints from earlier turns that the final message depends on.
- Preserve the user's original language and terminology. Do not translate.
- Do not invent facts, sources, dates, or details not present in the conversation.
- If the final user message is already fully self-contained, return it unchanged (modulo trivial cleanup).
- Drop greetings, thanks, and meta questions about the assistant itself; keep only the information need.
- Keep it concise — a search query, not a sentence. Aim for under 200 characters when possible.

Return only the rewritten query.`;function ce(r){let e="message";const t=[];for(const s of r.split(`
`)){const i=s.endsWith("\r")?s.slice(0,-1):s;if(i===""||i.startsWith(":"))continue;const n=i.indexOf(":"),a=n===-1?i:i.slice(0,n);let l=n===-1?"":i.slice(n+1);l.startsWith(" ")&&(l=l.slice(1)),a==="event"?e=l:a==="data"&&t.push(l)}return t.length===0?null:{event:e,data:t.join(`
`)}}function C(r){return r!==null&&typeof r=="object"&&!Array.isArray(r)}function K(...r){const e={};for(const t of r)if(t)for(const[s,i]of Object.entries(t)){const n=e[s];C(n)&&C(i)?e[s]=K(n,i):e[s]=i}return e}function he(r,e){if(!C(e))return r;const t=new URLSearchParams;for(const[h,d]of Object.entries(e))d!=null&&t.append(h,String(d));const s=t.toString();if(!s)return r;const i=r.indexOf("#"),n=i===-1?r:r.slice(0,i),a=i===-1?"":r.slice(i),l=n.includes("?")?"&":"?";return`${n}${l}${s}${a}`}function de(r){if(!C(r))return{};const e={};for(const[t,s]of Object.entries(r))s!=null&&(e[t]=String(s));return e}function pe(r){return C(r)?r:void 0}class ue{constructor(e){M(this,"activeRequests",new Map),M(this,"baseUrl"),this.baseUrl=e.replace(/\/$/,"")}request(e,t,s,i){const n=t==="search"?"snippet-search":"snippet-chat-completions",a=he(`${this.baseUrl}/${t}`,i?.queryParams);return fetch(a,{method:"POST",body:JSON.stringify(K(pe(i?.body),e)),headers:{...de(i?.headers),"Content-Type":"application/json",Accept:e.stream?"text/event-stream":"application/json","cf-ai-search-source":n},signal:s})}async search(e,t={}){const s=this.generateRequestId(),i=new AbortController,n=t.signal||i.signal;this.registerRequest(s,i);try{const a=await this.request({messages:[{role:"user",content:e}],stream:!1,ai_search_options:{retrieval:{metadata_only:!0,max_num_results:t.maxResults??30}}},"search",n,t.request);if(!a.ok)throw new Error(`HTTP error! status: ${a.status}`);if(!a.body)throw new Error("Response body is empty");const l=await a.json();if(l.success&&l.result)return l.result.chunks.map(h=>({type:"result",id:h.id,title:$(h.item.metadata?.title),description:h.item.metadata?.description?$(h.item.metadata?.description):"",timestamp:h.item.timestamp??void 0,url:h.item.key,image:h.item.metadata?.image||void 0,metadata:{...h.item.metadata,instance_id:h.instance_id}}));throw l.success===!1?new Error(l.error):new Error("Unknown error")}finally{this.unregisterRequest(s)}}async*searchStream(e,t={}){const s=this.generateRequestId(),i=new AbortController,n=t.signal||i.signal;this.registerRequest(s,i);const a=await this.request({messages:[{role:"user",content:e}],stream:!0,...t.maxResults!==void 0&&{max_num_results:t.maxResults}},"ai-search",n,t.request);if(!a.ok)throw new Error(`HTTP error! status: ${a.status}`);if(!a.body)throw new Error("Response body is empty");let l="";const h=a.body.getReader(),d=new TextDecoder;for(;;){const{done:p,value:u}=await h.read();if(p)break;const m=d.decode(u,{stream:!0});l+=m}yield{type:"result",id:"",title:"",description:l.replaceAll("data: ","").trim().split(`

`).map(p=>JSON.parse(p)).map(p=>p.response).join(""),url:"",metadata:{}}}async*chat(e,t){const s=new AbortController,i=t?.signal||s.signal,n=t?.stream??!0,a={messages:[...t?.history??[],{role:"user",content:e}],stream:n};if(t?.queryRewrite){const h=typeof t.queryRewrite=="object"?t.queryRewrite:{};a.ai_search_options={query_rewrite:{enabled:!0,model:h.model??oe,rewrite_prompt:h.rewritePrompt??le}}}const l=await this.request(a,"chat/completions",i);if(!l.ok)throw new Error(`HTTP error! status: ${l.status}`);if(!l.body)throw new Error("Response body is empty");if(!n){yield{type:"text",message:(await l.json()).choices.map(h=>h.message.content).join("")};return}yield*this.parseChatStream(l.body)}async*parseChatStream(e){const t=e.getReader(),s=new TextDecoder;let i="";const n=a=>{const l=ce(a);if(!l||l.event==="chunks")return null;if(l.data==="[DONE]")return"done";try{const h=JSON.parse(l.data).choices?.[0]?.delta?.content;if(typeof h=="string"&&h.length>0)return{type:"text",message:h}}catch(h){console.error("AISearchClient: failed to parse SSE chat chunk",h)}return null};try{for(;;){const{done:a,value:l}=await t.read();if(a)break;i+=s.decode(l,{stream:!0});let h=i.indexOf(`

`);for(;h!==-1;){const d=i.slice(0,h);i=i.slice(h+2);const p=n(d);if(p==="done")return;p&&(yield p),h=i.indexOf(`

`)}}if(i+=s.decode(),i.trim().length>0){const a=n(i);a&&a!=="done"&&(yield a)}}catch(a){if(a.name==="AbortError")return;throw a}finally{t.releaseLock()}}cancelRequest(e){const t=this.activeRequests.get(e);t&&(t.controller.abort(),this.unregisterRequest(e))}cancelAllRequests(){for(const[e]of this.activeRequests)this.cancelRequest(e)}registerRequest(e,t){this.activeRequests.set(e,{id:e,controller:t,timestamp:Date.now()})}unregisterRequest(e){this.activeRequests.delete(e)}generateRequestId(){return`req-${Date.now()}-${Math.random().toString(36).substr(2,9)}`}}const V=2500;function G(r,e){let t;function s(...i){clearTimeout(t),t=setTimeout(()=>{r(...i)},e)}return s.cancel=()=>clearTimeout(t),s}function c(r){const e=document.createElement("div");return e.textContent=r,e.innerHTML}function J(r){try{return decodeURI(r)}catch{return r}}function $(r){return new DOMParser().parseFromString(r,"text/html").documentElement.textContent||""}function _(r){return new Date(r).toLocaleDateString(void 0,{month:"short",day:"numeric"})}function g(r,e){return r!==null?r:e}function v(r,e){return r===null?e:r==="true"||r===""}function w(r,e){if(r===null)return e;const t=Number.parseInt(r,10);return Number.isNaN(t)?e:t}function Q(r){return typeof r!="string"||!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(r)?"":`<span class="result-icon" part="result-icon result-icon-${r}" aria-hidden="true"></span>`}function k(r,e){return new CustomEvent(r,{detail:e,bubbles:!0,composed:!0,cancelable:!0})}function Y(r,e,t="Other"){const s=new Map;for(const i of r){const n=i.metadata?.[e],a=n==null||String(n).trim()===""?t:String(n),l=s.get(a);l?l.push(i):s.set(a,[i])}return[...s].map(([i,n])=>({key:i,results:n}))}function Z(r,e){let t=0;return r.map((s,i)=>`
        <div class="result-group" role="group" aria-labelledby="result-group-label-${i}">
          <div class="result-group-header" id="result-group-label-${i}" role="presentation">${c(s.key)}</div>
          ${s.results.map(n=>e(n,t++)).join("")}
        </div>
      `).join("")}function W(r){if(!r)throw new Error("API URL is required");return new ue(r)}const me=/^[a-z][a-z0-9+.-]*:/i,ge=new Set(["http:","https:","mailto:","tel:"]);function I(r){if(typeof r!="string")return"";const e=r.replace(/^[\s\u0000-\u001f\u007f]+|[\s\u0000-\u001f\u007f]+$/g,"");if(e==="")return"";if(!me.test(e))return e;let t;try{t=new URL(e)}catch{return""}return ge.has(t.protocol)?e:""}var ve=Object.defineProperty,fe=(r,e,t)=>e in r?ve(r,e,{enumerable:!0,configurable:!0,writable:!0,value:t}):r[e]=t,o=(r,e,t)=>fe(r,typeof e!="symbol"?e+"":e,t);const be=2e4,ye=20,we="/stats";function z(){return typeof document<"u"&&typeof window<"u"}class X{constructor(e,t={}){o(this,"baseUrl"),o(this,"endpoint"),o(this,"snippetVersion"),o(this,"flushIntervalMs"),o(this,"maxBufferSize"),o(this,"buffer",[]),o(this,"flushTimer",null),o(this,"destroyed",!1),o(this,"boundUnloadHandler"),o(this,"boundVisibilityHandler"),this.baseUrl=e.replace(/\/$/,""),this.endpoint=t.endpoint??we,this.snippetVersion=t.snippetVersion??ie,this.flushIntervalMs=t.flushIntervalMs??be,this.maxBufferSize=Math.max(1,t.maxBufferSize??ye),this.boundUnloadHandler=()=>this.flushBeacon(),this.boundVisibilityHandler=()=>{typeof document<"u"&&document.visibilityState==="hidden"&&this.flushBeacon()},z()&&(window.addEventListener("pagehide",this.boundUnloadHandler),document.addEventListener("visibilitychange",this.boundVisibilityHandler))}trackSearch(e,t){this.track({inputQuery:e,snippetVersion:this.snippetVersion,totalResult:t})}trackClick(e,t,s,i){this.track({inputQuery:e,snippetVersion:this.snippetVersion,totalResult:t,clickedResultId:s,clickPosition:i,clickViewMore:!1})}trackViewMore(e,t){this.track({inputQuery:e,snippetVersion:this.snippetVersion,totalResult:t,clickViewMore:!0})}track(e){if(!this.destroyed){if(this.buffer.push(e),this.buffer.length>=this.maxBufferSize){this.flush();return}this.scheduleFlush()}}flush(){const e=this.drainBuffer();if(e.length===0)return;const t=JSON.stringify({events:e});fetch(this.buildUrl(),{method:"POST",headers:{"Content-Type":"application/json"},body:t,keepalive:!0}).catch(s=>{console.log(s)})}flushBeacon(){const e=this.drainBuffer();if(e.length===0)return;const t=JSON.stringify({events:e}),s=this.buildUrl();if(typeof navigator<"u"&&typeof navigator.sendBeacon=="function")try{const i=new Blob([t],{type:"application/json"});if(navigator.sendBeacon(s,i))return}catch{}typeof fetch<"u"&&fetch(s,{method:"POST",headers:{"Content-Type":"application/json"},body:t,keepalive:!0}).catch(()=>{})}destroy(){this.destroyed||(this.destroyed=!0,this.flushTimer!==null&&(clearTimeout(this.flushTimer),this.flushTimer=null),z()&&(window.removeEventListener("pagehide",this.boundUnloadHandler),document.removeEventListener("visibilitychange",this.boundVisibilityHandler)),this.flushBeacon())}scheduleFlush(){this.flushTimer!==null||this.destroyed||(this.flushTimer=setTimeout(()=>{this.flushTimer=null,this.flush()},this.flushIntervalMs))}drainBuffer(){if(this.flushTimer!==null&&(clearTimeout(this.flushTimer),this.flushTimer=null),this.buffer.length===0)return[];const e=this.buffer;return this.buffer=[],e}buildUrl(){const e=this.endpoint.startsWith("/")?this.endpoint:`/${this.endpoint}`;return`${this.baseUrl}${e}`}}const xe=`
/* Search view states */
.search-view {
  transition: var(--search-snippet-transition-slow);
  background: var(--search-snippet-background);
  border-radius: var(--search-snippet-border-radius);
  padding: 0px;
}

.search-view-collapsed {
  max-height: 60px;
}

.search-view-expanded {
  max-height: var(--search-snippet-max-height);
}


.search-icon {
  width: var(--search-snippet-icon-size);
  height: var(--search-snippet-icon-size);
  margin-left: var(--search-snippet-icon-margin-left);
  color: var(--search-snippet-text-color);
}

/* Search input wrapper */
.search-input-wrapper {
  display: grid;
  grid-template-columns: auto 1fr auto;
  align-items: center;
  gap: var(--search-snippet-spacing-sm);
  overflow: hidden;
  transition: max-width var(--search-snippet-transition-slow), 
              opacity var(--search-snippet-transition);
  padding: var(--search-snippet-spacing-sm);
  border-radius: var(--search-snippet-border-radius);
  border: var(--search-snippet-border-width) solid var(--search-snippet-border-color);
}



.search-input {
  flex: 1;
  border: none;
  outline: none;
  background: transparent;
  color: var(--search-snippet-text-color);
  font-size: var(--search-snippet-font-size-base);
  font-weight: var(--search-snippet-font-weight-medium);
  box-shadow: none;
  padding: 0;
}

.search-input::placeholder {
  color: var(--search-snippet-text-secondary);
}

.search-view:has(.search-input:not(:placeholder-shown)) .search-input-wrapper, .search-view:has(.search-input:not(:placeholder-shown)) {
  border-bottom-left-radius: 0;
  border-bottom-right-radius: 0;
}

.search-view:focus-within {
  border-color: var(--search-snippet-primary-color);
  box-shadow: inset 0 0 0 3px var(--search-snippet-focus-ring);
}

.search-view:has(.search-input:not(:placeholder-shown)) .search-content {
  max-height: 600px;
  opacity: 1;
  overflow-y: auto;
  padding: 8px;
}

.search-submit-button {
  flex-shrink: 0;
  
  border-radius: max(var(--search-snippet-button-min-border-radius, 4px), calc(var(--search-snippet-border-radius) - var(--search-snippet-spacing-sm)))
}

/* Search content */
.search-content {
  max-height: 0;
  opacity: 0;
  transition: max-height var(--search-snippet-transition-slow),
              opacity var(--search-snippet-transition);
  position: absolute;
  width: 100%;
  background: var(--search-snippet-background);
  border-bottom-left-radius: var(--search-snippet-border-radius);
  border-bottom-right-radius: var(--search-snippet-border-radius);
  border: var(--search-snippet-border-width) solid var(--search-snippet-border-color);
  border-top: none;
}

.search-content::-webkit-scrollbar {
  width: 8px;
  height: 100px;
}

.search-content::-webkit-scrollbar-track {
  background: var(--search-snippet-surface);
  
}

.search-content::-webkit-scrollbar-thumb {
  background: var(--search-snippet-border-color);
  border-radius: var(--search-snippet-border-radius);
}

.search-content::-webkit-scrollbar-thumb:hover {
  background: var(--search-snippet-text-secondary);
}

.container {
  overflow: unset;
  position: relative;
  border: none;
}

.container:has(.search-input:not(:placeholder-shown)) {
  border-bottom-left-radius: 0;
  border-bottom-right-radius: 0;
}


/* Override header for search mode */

/* Search results */
.search-results {
  display: flex;
  flex-direction: column;
  gap: var(--search-snippet-spacing-sm);
}

.result-group {
  display: flex;
  flex-direction: column;
  gap: var(--search-snippet-spacing-sm);
}

.result-group + .result-group {
  margin-top: var(--search-snippet-spacing-md);
}

.result-group-header {
  padding: var(--search-snippet-spacing-xs) 0;
  color: var(--search-snippet-text-secondary);
  font-size: var(--search-snippet-font-size-sm);
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

a.search-result-item {
  display: flex;
  flex-direction: row;
  align-items: flex-start;
  gap: var(--search-snippet-spacing-md);
  padding: var(--search-snippet-spacing-md);
  background: var(--search-snippet-surface);
  border: var(--search-snippet-border-width) solid var(--search-snippet-border-color);
  border-radius: var(--search-snippet-border-radius);
  cursor: pointer;
  transition: var(--search-snippet-transition);
  text-decoration: none;
  color: inherit;
}

/* Image thumbnail container */
.search-result-image-container {
  flex-shrink: 0;
  width: 64px;
  height: 64px;
  border-radius: calc(var(--search-snippet-border-radius) - 4px);
  overflow: hidden;
  position: relative;
}

.search-result-image {
  width: 100%;
  height: 100%;
  object-fit: contain;
  opacity: 0;
  transition: opacity var(--search-snippet-transition);
}

.search-result-image.loaded {
  opacity: 1;
}

/* Loading shimmer */
.search-result-image-loading {
  position: absolute;
  inset: 0;
  background: linear-gradient(
    90deg,
    var(--search-snippet-surface) 25%,
    var(--search-snippet-border-color) 50%,
    var(--search-snippet-surface) 75%
  );
  background-size: 200% 100%;
  animation: search-image-shimmer 1.5s infinite;
}

@keyframes search-image-shimmer {
  0% { background-position: 200% 0; }
  100% { background-position: -200% 0; }
}

/* Placeholder icon */
.search-result-image-placeholder {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--search-snippet-text-secondary);
  opacity: 0.5;
}

.search-result-image-placeholder svg {
  width: 24px;
  height: 24px;
}

/* Content wrapper */
.search-result-content {
  flex: 1;
  min-width: 0;
}

a.search-result-item:hover {
  background: var(--search-snippet-hover-background);
  border-color: var(--search-snippet-primary-color);
  transform: translateY(-1px);
  box-shadow: var(--search-snippet-result-item-shadow);
}

a.search-result-item:focus-visible {
  outline: 2px solid var(--search-snippet-primary-color);
  outline-offset: 2px;
}

.search-result-title {
  font-size: var(--search-snippet-font-size-base);
  font-weight: var(--search-snippet-font-weight-medium);
  color: var(--search-snippet-text-color);
  margin-bottom: var(--search-snippet-spacing-xs);
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.search-result-snippet {
  font-size: var(--search-snippet-font-size-sm);
  color: var(--search-snippet-text-description);
  line-height: 1.6;
  display: -webkit-box;
  -webkit-line-clamp: 3;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.search-result-metadata {
  display: flex;
  align-items: center;
  gap: var(--search-snippet-spacing-sm);
  margin-top: var(--search-snippet-spacing-xs);
  min-width: 0;
}

.search-result-url {
  font-size: var(--search-snippet-font-size-sm);
  color: var(--search-snippet-primary-color);
  text-decoration: none;
  display: block;
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.search-result-url-empty {
  visibility: hidden;
}

.search-result-date {
  font-size: 12px;
  font-weight: var(--search-snippet-font-weight-medium);
  color: var(--search-snippet-text-secondary);
  text-align: right;
  flex-shrink: 0;
}

.search-result-url:hover {
  text-decoration: underline;
}

/* Search header */
.search-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: var(--search-snippet-spacing-md);
  padding-bottom: var(--search-snippet-spacing-sm);
  border-bottom: var(--search-snippet-border-width) solid var(--search-snippet-border-color);
}

.search-count {
  font-size: var(--search-snippet-font-size-sm);
  color: var(--search-snippet-text-secondary);
}

/* Search footer */
.search-footer {
  padding: var(--search-snippet-spacing-md);
  padding-bottom: var(--search-snippet-spacing-xs);
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--search-snippet-spacing-sm);
}

/* See more link */
.search-see-more {
  display: inline-flex;
  align-items: center;
  gap: var(--search-snippet-spacing-xs);
  font-size: var(--search-snippet-font-size-sm);
  color: var(--search-snippet-primary-color);
  text-decoration: none;
  font-weight: var(--search-snippet-font-weight-medium);
  transition: color var(--search-snippet-transition-fast);
}

.search-see-more:hover {
  text-decoration: underline;
}

/* Loading state for search */
.search-loading {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: var(--search-snippet-spacing-xxl);
  gap: var(--search-snippet-spacing-md);
  color: var(--search-snippet-text-secondary);
}

/* Empty search state */
.search-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: var(--search-snippet-spacing-xxl);
  gap: var(--search-snippet-spacing-md);
  color: var(--search-snippet-text-secondary);
  text-align: center;
}

.search-empty-icon {
  width: 64px;
  height: 64px;
  opacity: 0.5;
}

.search-empty-title {
  font-size: var(--search-snippet-font-size-lg);
  font-weight: var(--search-snippet-font-weight-medium);
  color: var(--search-snippet-text-color);
}

.search-empty-description {
  font-size: var(--search-snippet-font-size-sm);
}

/* Highlight matching text */
.search-highlight {
  background: var(--search-snippet-warning-background);
  color: var(--search-snippet-warning-color);
  padding: 1px 2px;
  border-radius: 2px;
  font-weight: var(--search-snippet-font-weight-medium);
}
`,q="search-bar-snippet",B=10,O=50;class ke extends HTMLElement{constructor(){super(),o(this,"shadow"),o(this,"client",null),o(this,"stats",null),o(this,"container",null),o(this,"inputElement",null),o(this,"resultsContainer",null),o(this,"searchButton",null),o(this,"debouncedSearch",null),o(this,"currentSearchController",null),o(this,"loadingMessageInterval",null),o(this,"loadingMessageIndex",0),o(this,"translationsOverride",null),o(this,"resolvedTranslations",f(null)),o(this,"lastSearchQuery",""),o(this,"lastSearchTotal",0),o(this,"handleInputChange",null),o(this,"handleInputKeydownEnter",null),o(this,"handleInputKeydownEscape",null),o(this,"handleSearchButtonClick",null),o(this,"handleResultClick",null),o(this,"handleSeeMoreClick",null),this.shadow=this.attachShadow({mode:"open"})}static get observedAttributes(){return["api-url","placeholder","max-results","max-render-results","debounce-ms","theme","hide-branding","show-url","show-date","hide-thumbnails","see-more","disable-analytics","request-options","translations"]}connectedCallback(){this.syncTranslationsFromAttribute(),this.initializeClient(),this.render(),this.dispatchEvent(k("ready",void 0))}disconnectedCallback(){this.cleanup()}attributeChangedCallback(e,t,s){t!==s&&(e==="api-url"||e==="disable-analytics"?this.initializeClient():e==="theme"?this.updateTheme(s):e==="translations"&&(this.syncTranslationsFromAttribute(),this.isConnected&&this.rerender()))}get translations(){return this.translationsOverride}set translations(e){this.translationsOverride=e??null,this.resolvedTranslations=f(this.translationsOverride),this.isConnected&&this.rerender()}rerender(){const e=this.inputElement?.value??"";this.render(),e&&this.inputElement&&(this.inputElement.value=e,e.trim().length>0&&this.performSearch(e.trim()))}syncTranslationsFromAttribute(){if(this.translationsOverride){this.resolvedTranslations=f(this.translationsOverride);return}const e=D(this.getAttribute("translations"),"SearchBarSnippet");this.resolvedTranslations=f(e)}getProps(){const e=this.resolvedTranslations;return{apiUrl:g(this.getAttribute("api-url"),""),placeholder:g(this.getAttribute("placeholder"),e.placeholder),maxResults:w(this.getAttribute("max-results"),O),maxRenderResults:w(this.getAttribute("max-render-results"),B),debounceMs:w(this.getAttribute("debounce-ms"),300),theme:g(this.getAttribute("theme"),"auto"),hideBranding:v(this.getAttribute("hide-branding"),!1),showUrl:v(this.getAttribute("show-url"),!1),showDate:v(this.getAttribute("show-date"),!1),hideThumbnails:v(this.getAttribute("hide-thumbnails"),!1),seeMore:g(this.getAttribute("see-more"),""),groupBy:g(this.getAttribute("group-by"),""),disableAnalytics:v(this.getAttribute("disable-analytics"),!1),translations:this.translationsOverride??void 0}}getRequestOptions(){const e=this.getAttribute("request-options");if(e)try{const t=JSON.parse(e);if(t===null||typeof t!="object"||Array.isArray(t))throw new Error("request-options must be a JSON object");return t}catch(t){console.error("SearchBarSnippet: invalid request-options attribute",t);return}}initializeClient(){const e=this.getProps();if(!e.apiUrl){console.error("SearchBarSnippet: api-url attribute is required"),this.client=null,this.destroyStatsClient(),this.showMissingApiUrlError();return}try{this.client=W(e.apiUrl),this.destroyStatsClient(),e.disableAnalytics||(this.stats=new X(e.apiUrl))}catch(t){console.error("SearchBarSnippet:",t)}}destroyStatsClient(){this.stats&&(this.stats.destroy(),this.stats=null)}render(){const e=this.getProps(),t=this.resolvedTranslations,s=n=>this.performSearch(n);this.debouncedSearch=G(s,e.debounceMs||400);const i=document.createElement("style");i.textContent=`${F}
${xe}`,this.container=document.createElement("div"),this.container.className="container",this.container.innerHTML=`
            <div class="search-view"> 
                <div class="search-input-wrapper">
                    <svg xmlns="http://www.w3.org/2000/svg" class="search-icon" height="24px" viewBox="0 -960 960 960" width="24px" fill="currentColor"><path d="M784-120 532-372q-30 24-69 38t-83 14q-109 0-184.5-75.5T120-580q0-109 75.5-184.5T380-840q109 0 184.5 75.5T640-580q0 44-14 83t-38 69l252 252-56 56ZM380-400q75 0 127.5-52.5T560-580q0-75-52.5-127.5T380-760q-75 0-127.5 52.5T200-580q0 75 52.5 127.5T380-400Z"/></svg>
                    <input
                        type="text"
                        name="search-input"
                        class="search-input"
                        placeholder="${c(e.placeholder||t.placeholder)}"
                        aria-label="${c(t.searchInputAriaLabel)}"
                        autocomplete="off"
                    />
                    <button class="button search-submit-button" aria-label="${c(t.searchButtonLabel)}">
                        <span>${c(t.searchButtonLabel)}</span>
                    </button>
                </div>
                <div class="search-content">
                    <div class="search-results-wrapper">
                        <!-- Results will be inserted here -->
                    </div>
                </div>
            </div>
        `,this.shadow.innerHTML="",this.shadow.appendChild(i),this.shadow.appendChild(this.container),this.inputElement=this.container.querySelector(".search-input"),this.resultsContainer=this.container.querySelector(".search-results-wrapper"),this.searchButton=this.container.querySelector(".search-submit-button"),this.attachEventListeners(),this.client||this.showMissingApiUrlError()}attachEventListeners(){this.inputElement&&(this.handleInputChange=e=>{const t=e.target.value.trim();t.length>0&&this.debouncedSearch?this.debouncedSearch(t):this.showEmptyState()},this.inputElement.addEventListener("input",this.handleInputChange),this.handleInputKeydownEnter=e=>{if(e.key==="Enter"){const t=e.target.value.trim();t.length>0&&this.performSearch(t)}},this.inputElement.addEventListener("keydown",this.handleInputKeydownEnter),this.handleInputKeydownEscape=e=>{e.key==="Escape"&&this.inputElement&&(this.inputElement.value="")},window.addEventListener("keydown",this.handleInputKeydownEscape),this.searchButton&&(this.handleSearchButtonClick=()=>{const e=this.inputElement?.value.trim()||"";e.length>0&&this.performSearch(e)},this.searchButton.addEventListener("click",this.handleSearchButtonClick)))}async performSearch(e){if(!this.client){this.showMissingApiUrlError();return}this.currentSearchController&&(this.currentSearchController.abort(),this.currentSearchController=null),this.currentSearchController=new AbortController,this.showLoadingState();try{const t=this.getProps(),s=await this.client.search(e,{signal:this.currentSearchController.signal,maxResults:t.maxResults||O,request:this.getRequestOptions()}),i=s.slice(0,t.maxRenderResults||B);this.lastSearchQuery=e,this.lastSearchTotal=s.length,this.stats?.trackSearch(e,s.length),this.displayResults(i,e,s.length)}catch(t){if(t.name==="AbortError")return;this.showErrorState(t.message)}finally{this.currentSearchController=null}}displayResults(e,t,s=e.length){if(this.clearLoadingInterval(),!this.resultsContainer)return;if(e.length===0){this.showNoResultsState(t);return}const i=this.getProps(),n=this.resolvedTranslations,a=i.hideBranding?"":`<div class="powered-by-inline">${P}</div>`,l=s>e.length,h=l?y(n.resultsCountOverflow,{n:e.length,total:s}):y(s===1?n.resultsCount:n.resultsCountPlural,{n:s}),d=i.seeMore&&l?`<div class="search-footer">
            <a href="${c(i.seeMore+encodeURIComponent(t))}" class="search-see-more">
              <span>${c(n.seeMoreResults)}</span>
              <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14"/><path d="m12 5 7 7-7 7"/></svg>
            </a>
          </div>`:"",p=`
            <div class="search-header">
                <div class="search-count">
                    ${c(h)}
                </div>
                ${a}
            </div>
            <div class="search-results">
                ${i.groupBy?Z(Y(e,i.groupBy,n.groupOther),(u,m)=>this.renderResult(u,m)):e.map((u,m)=>this.renderResult(u,m)).join("")}
            </div>
            ${d}
        `;this.resultsContainer.innerHTML=p,this.attachResultHandlers()}renderResult(e,t){const s=this.getProps(),i=Q(e.metadata?.icon),n=s.hideThumbnails?"":this.renderResultImage(e.image,e.title),a=I(e.url),l=a?c(a):"#",h=a?c(J(a)):"",d=s.showDate&&e.timestamp!==void 0?`<div class="search-result-date">${c(_(e.timestamp))}</div>`:"",p=s.showUrl&&a||d?`<div class="search-result-metadata">
            ${s.showUrl&&a?`<span class="search-result-url">${h}</span>`:'<span class="search-result-url search-result-url-empty"></span>'}
            ${d}
          </div>`:"";return`
            <a href="${l}" class="search-result-item" data-index="${t}" data-result-id="${c(e.id||"")}">
                ${i}
                ${n}
                <div class="search-result-content">
                    <div class="search-result-title">${c(e.title||"")}</div>
                    <div class="search-result-snippet">${c(e.description||"")}</div>
                    ${p}
                </div>
            </a>
        `}renderResultImage(e,t){const s='<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="16" y1="13" x2="8" y2="13"/><line x1="16" y1="17" x2="8" y2="17"/><polyline points="10 9 9 9 8 9"/></svg>';return e?`
      <div class="search-result-image-container">
        <div class="search-result-image-loading"></div>
        <div class="search-result-image-placeholder" style="display: none;">${s}</div>
        <img 
          class="search-result-image" 
          src="${c(e)}" 
          alt="${c(t)}"
          loading="lazy"
        />
      </div>
    `:`
        <div class="search-result-image-container">
          <div class="search-result-image-placeholder">${s}</div>
        </div>
      `}attachResultHandlers(){this.detachResultTrackingHandlers();const e=this.resultsContainer;if(!e)return;this.handleResultClick=s=>{const i=s.target?.closest(".search-result-item");if(!i)return;i.getAttribute("href")==="#"&&s.preventDefault();const n=i.getAttribute("data-index"),a=i.getAttribute("data-result-id")??"",l=n!==null?Number.parseInt(n,10):Number.NaN;!Number.isNaN(l)&&a&&this.stats?.trackClick(this.lastSearchQuery,this.lastSearchTotal,a,l)},e.addEventListener("click",this.handleResultClick);const t=e.querySelector(".search-see-more");t&&(this.handleSeeMoreClick=()=>{this.stats?.trackViewMore(this.lastSearchQuery,this.lastSearchTotal)},t.addEventListener("click",this.handleSeeMoreClick)),this.container?.querySelectorAll(".search-result-image")?.forEach(s=>{s.addEventListener("load",()=>{s.classList.add("loaded"),s.closest(".search-result-image-container")?.querySelector(".search-result-image-loading")?.remove()}),s.addEventListener("error",()=>{const i=s.closest(".search-result-image-container");i?.querySelector(".search-result-image-loading")?.remove();const n=i?.querySelector(".search-result-image-placeholder");n&&(n.style.display="flex"),s.style.display="none"})})}detachResultTrackingHandlers(){const e=this.resultsContainer;e&&this.handleResultClick&&e.removeEventListener("click",this.handleResultClick),this.handleResultClick=null,this.handleSeeMoreClick=null}showLoadingState(){if(!this.resultsContainer)return;this.clearLoadingInterval();const e=this.resolvedTranslations.loadingMessages;this.loadingMessageIndex=Math.floor(Math.random()*e.length);const t=this.resolvedTranslations;this.resultsContainer.innerHTML=`
            <div class="search-loading">
                <div class="loading" aria-label="${c(t.loadingAriaLabel)}"></div>
                <div class="loading-text loading-text-animate">${c(e[this.loadingMessageIndex])}</div>
            </div>
        `,this.startLoadingInterval()}startLoadingInterval(){this.loadingMessageInterval=setInterval(()=>{const e=this.resolvedTranslations.loadingMessages;this.loadingMessageIndex=(this.loadingMessageIndex+1)%e.length;const t=this.resultsContainer?.querySelector(".loading-text");t&&(t.classList.remove("loading-text-animate"),t.offsetWidth,t.textContent=e[this.loadingMessageIndex],t.classList.add("loading-text-animate"))},V)}clearLoadingInterval(){this.loadingMessageInterval&&(clearInterval(this.loadingMessageInterval),this.loadingMessageInterval=null)}showEmptyState(){if(this.clearLoadingInterval(),!this.resultsContainer)return;const e=this.resolvedTranslations;this.resultsContainer.innerHTML=`
            <div class="search-empty">
                <svg class="search-empty-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <circle cx="11" cy="11" r="8"></circle>
                    <path d="m21 21-4.35-4.35"></path>
                </svg>
                <div class="search-empty-title">${c(e.emptyStateTitle)}</div>
                <div class="search-empty-description">
                    ${c(e.emptyStateDescription)}
                </div>
            </div>
        `}showNoResultsState(e){if(this.clearLoadingInterval(),!this.resultsContainer)return;const t=this.resolvedTranslations;this.resultsContainer.innerHTML=`
            <div class="search-empty">
                <svg class="search-empty-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <circle cx="11" cy="11" r="8"></circle>
                    <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
                </svg>
                <div class="search-empty-title">${c(t.noResultsTitle)}</div>
                <div class="search-empty-description">
                    ${c(y(t.noResultsDescription,{query:e}))}
                </div>
            </div>
        `}showErrorState(e){if(this.clearLoadingInterval(),!this.resultsContainer)return;const t=this.resolvedTranslations;this.resultsContainer.innerHTML=`
            <div class="error">
                <strong>${c(t.errorPrefix)}</strong> ${c(e)}
            </div>
        `}showMissingApiUrlError(){this.resultsContainer&&this.showErrorState(this.resolvedTranslations.missingApiUrlError)}updateTheme(e){const t=e==="light"||e==="dark"||e==="auto"?e:"auto";t==="auto"?this.removeAttribute("theme"):this.setAttribute("theme",t)}cleanup(){this.clearLoadingInterval(),this.currentSearchController&&(this.currentSearchController.abort(),this.currentSearchController=null),this.client&&this.client.cancelAllRequests(),this.destroyStatsClient(),this.inputElement&&(this.handleInputChange&&this.inputElement.removeEventListener("input",this.handleInputChange),this.handleInputKeydownEnter&&this.inputElement.removeEventListener("keydown",this.handleInputKeydownEnter),this.handleInputKeydownEscape&&window.removeEventListener("keydown",this.handleInputKeydownEscape)),this.searchButton&&this.handleSearchButtonClick&&this.searchButton.removeEventListener("click",this.handleSearchButtonClick),this.detachResultTrackingHandlers(),this.handleInputChange=null,this.handleInputKeydownEnter=null,this.handleInputKeydownEscape=null,this.handleSearchButtonClick=null}async search(e){await this.performSearch(e)}}customElements.get(q)||customElements.define(q,ke);const Se=`
/* Modal backdrop */
.modal-backdrop {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.5);
  backdrop-filter: blur(4px);
  z-index: var(--search-snippet-z-modal);
  opacity: 0;
  visibility: hidden;
  transition: opacity var(--search-snippet-transition), visibility var(--search-snippet-transition);
}

.modal-backdrop.open {
  opacity: 1;
  visibility: visible;
}

/* Modal container */
.modal-container {
  position: fixed;
  top: 15%;
  left: 50%;
  transform: translateX(-50%) scale(0.95);
  width: 90%;
  max-width: 600px;
  max-height: 70vh;
  background: var(--search-snippet-background);
  border: var(--search-snippet-border-width) solid var(--search-snippet-border-color);
  border-radius: var(--search-snippet-border-radius);
  box-shadow: var(--search-snippet-shadow-lg);
  z-index: calc(var(--search-snippet-z-modal) + 1);
  display: flex;
  flex-direction: column;
  opacity: 0;
  visibility: hidden;
  transition: opacity var(--search-snippet-transition), 
              visibility var(--search-snippet-transition),
              transform var(--search-snippet-transition);
}

.modal-container.open {
  opacity: 1;
  visibility: visible;
  transform: translateX(-50%) scale(1);
}

/* Modal header with search input */
.modal-header {
  display: flex;
  align-items: center;
  gap: var(--search-snippet-spacing-sm);
  padding: var(--search-snippet-spacing-md);
  border-bottom: var(--search-snippet-border-width) solid var(--search-snippet-border-color);
}

.modal-search-icon {
  width: var(--search-snippet-icon-size);
  height: var(--search-snippet-icon-size);
  color: var(--search-snippet-text-secondary);
  flex-shrink: 0;
}

.modal-search-input {
  flex: 1;
  border: none;
  outline: none;
  background: transparent;
  color: var(--search-snippet-text-color);
  font-size: var(--search-snippet-font-size-lg);
  font-family: var(--search-snippet-font-family);
  font-weight: var(--search-snippet-font-weight-normal);
  padding: var(--search-snippet-spacing-xs) 0;
}

.modal-search-input::placeholder {
  color: var(--search-snippet-text-secondary);
}

.modal-shortcut-hint {
  display: flex;
  align-items: center;
  gap: var(--search-snippet-spacing-xs);
  color: var(--search-snippet-text-secondary);
  font-size: var(--search-snippet-font-size-sm);
  flex-shrink: 0;
}

.modal-kbd {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 24px;
  height: 22px;
  padding: 0 var(--search-snippet-spacing-xs);
  background: var(--search-snippet-surface);
  border: var(--search-snippet-border-width) solid var(--search-snippet-border-color);
  border-radius: 4px;
  font-size: var(--search-snippet-font-size-sm);
  font-family: var(--search-snippet-font-family);
  color: var(--search-snippet-text-secondary);
}

/* Modal content (results area) */
.modal-content {
  flex: 1;
  overflow-y: auto;
  overflow-x: hidden;
  padding: var(--search-snippet-spacing-sm);
}

.modal-content::-webkit-scrollbar {
  width: 8px;
}

.modal-content::-webkit-scrollbar-track {
  background: var(--search-snippet-surface);
}

.modal-content::-webkit-scrollbar-thumb {
  background: var(--search-snippet-border-color);
  border-radius: var(--search-snippet-border-radius);
}

.modal-content::-webkit-scrollbar-thumb:hover {
  background: var(--search-snippet-text-secondary);
}

/* Results list */
.modal-results {
  display: flex;
  flex-direction: column;
  gap: var(--search-snippet-spacing-xs);
}

.result-group {
  display: flex;
  flex-direction: column;
  gap: var(--search-snippet-spacing-xs);
}

.result-group + .result-group {
  margin-top: var(--search-snippet-spacing-sm);
}

.result-group-header {
  position: sticky;
  top: 0;
  z-index: 1;
  padding: var(--search-snippet-spacing-xs) var(--search-snippet-spacing-md);
  background: var(--search-snippet-surface);
  color: var(--search-snippet-text-secondary);
  font-size: var(--search-snippet-font-size-sm);
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

/*
 * Mask the scroll container's top padding while the header is pinned so
 * result items don't peek through as a sliver between the search input and
 * the (sticky) category header. The pseudo-element rides above the header,
 * painted with the modal background, covering the padding gap and giving a
 * clean cutoff for items scrolling underneath.
 */
.result-group-header::before {
  content: "";
  position: absolute;
  left: 0;
  right: 0;
  bottom: 100%;
  height: var(--search-snippet-spacing-sm);
  background: var(--search-snippet-background);
}

.modal-result-row {
  position: relative;
  display: flex;
}


a.modal-result-item {
  flex: 1;
  min-width: 0;
  padding: var(--search-snippet-spacing-md);
  padding-right: calc(
    36px + var(--search-snippet-spacing-md) + var(--search-snippet-spacing-md)
  );
  background: transparent;
  border: var(--search-snippet-border-width) solid transparent;
  border-radius: calc(var(--search-snippet-border-radius) - 4px);
  cursor: pointer;
  transition: var(--search-snippet-transition-fast);
  display: flex;
  flex-direction: row;
  align-items: flex-start;
  gap: var(--search-snippet-spacing-md);
  text-decoration: none;
  color: inherit;
}

/* Image thumbnail container */
.modal-result-image-container {
  flex-shrink: 0;
  width: 48px;
  height: 48px;
  border-radius: 6px;
  overflow: hidden;
  position: relative;
}

.modal-result-image {
  width: 100%;
  height: 100%;
  object-fit: contain;
  opacity: 0;
  transition: opacity var(--search-snippet-transition);
}

.modal-result-image.loaded {
  opacity: 1;
}

/* Loading shimmer */
.modal-result-image-loading {
  position: absolute;
  inset: 0;
  background: linear-gradient(
    90deg,
    var(--search-snippet-surface) 25%,
    var(--search-snippet-border-color) 50%,
    var(--search-snippet-surface) 75%
  );
  background-size: 200% 100%;
  animation: modal-image-shimmer 1.5s infinite;
}

@keyframes modal-image-shimmer {
  0% { background-position: 200% 0; }
  100% { background-position: -200% 0; }
}

/* Placeholder icon */
.modal-result-image-placeholder {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--search-snippet-text-secondary);
  opacity: 0.5;
}

.modal-result-image-placeholder svg {
  width: 20px;
  height: 20px;
}

/* Content wrapper */
.modal-result-content {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: var(--search-snippet-spacing-xs);
}

a.modal-result-item:hover,
a.modal-result-item.active {
  background: var(--search-snippet-hover-background);
  border-color: var(--search-snippet-border-color);
}

a.modal-result-item.active {
  border-color: var(--search-snippet-primary-color);
  background: var(--search-snippet-focus-ring);
}

a.modal-result-item:focus-visible {
  outline: 2px solid var(--search-snippet-primary-color);
  outline-offset: -2px;
}

.modal-favorite-button {
  position: absolute;
  top: 50%;
  right: var(--search-snippet-spacing-md);
  z-index: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  padding: 8px;
  transform: translateY(-50%);
  border: 0;
  border-radius: 50%;
  background: transparent;
  color: var(--search-snippet-text-secondary);
  cursor: pointer;
  transition: var(--search-snippet-transition-fast);
}

.modal-favorite-button:hover,
.modal-favorite-button:focus-visible {
  background: var(--search-snippet-hover-background);
  color: var(--search-snippet-primary-color);
}

.modal-favorite-button:focus-visible {
  outline: 2px solid var(--search-snippet-primary-color);
  outline-offset: -2px;
}

.modal-favorite-button svg {
  width: 20px;
  height: 20px;
  fill: none;
  stroke: currentColor;
  stroke-linejoin: round;
  stroke-width: 1.75;
}

.modal-favorite-button.favorite {
  color: var(--search-snippet-primary-color);
}

.modal-favorite-button.favorite svg {
  fill: currentColor;
}

.modal-result-title {
  font-size: var(--search-snippet-font-size-base);
  font-weight: var(--search-snippet-font-weight-medium);
  color: var(--search-snippet-text-color);
  display: -webkit-box;
  -webkit-line-clamp: 1;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.modal-result-description {
  font-size: var(--search-snippet-font-size-sm);
  color: var(--search-snippet-text-description);
  line-height: 1.5;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.modal-result-metadata {
  display: flex;
  align-items: center;
  gap: var(--search-snippet-spacing-sm);
  min-width: 0;
}

.modal-result-url {
  font-size: var(--search-snippet-font-size-sm);
  color: var(--search-snippet-primary-color);
  text-decoration: none;
  display: block;
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.modal-result-url-empty {
  visibility: hidden;
}

.modal-result-date {
  font-size: 12px;
  font-weight: var(--search-snippet-font-weight-medium);
  color: var(--search-snippet-text-secondary);
  text-align: right;
  flex-shrink: 0;
}

.modal-result-url:hover {
  text-decoration: underline;
}

/* Initial modal sections */
.modal-initial-section {
  display: flex;
  flex-direction: column;
  gap: var(--search-snippet-spacing-xs);
}

.modal-initial-section + .modal-initial-section {
  margin-top: var(--search-snippet-spacing-md);
}

.modal-initial-section-title {
  display: flex;
  align-items: center;
  gap: var(--search-snippet-spacing-xs);
  margin: 0;
  padding: var(--search-snippet-spacing-xs) var(--search-snippet-spacing-md);
  color: var(--search-snippet-text-secondary);
  font-size: var(--search-snippet-font-size-sm);
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.modal-initial-section-title svg {
  width: 16px;
  height: 16px;
  fill: none;
  stroke: currentColor;
  stroke-linecap: round;
  stroke-linejoin: round;
  stroke-width: 1.75;
}

/* Result group header */
.modal-group-header {
  padding: var(--search-snippet-spacing-sm) var(--search-snippet-spacing-md);
  font-size: var(--search-snippet-font-size-sm);
  font-weight: var(--search-snippet-font-weight-medium);
  color: var(--search-snippet-text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

/* Loading state */
.modal-loading {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: var(--search-snippet-spacing-xxl);
  gap: var(--search-snippet-spacing-md);
  color: var(--search-snippet-text-secondary);
}

/* Empty state */
.modal-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: var(--search-snippet-spacing-xxl);
  gap: var(--search-snippet-spacing-md);
  color: var(--search-snippet-text-secondary);
  text-align: center;
}

.modal-empty-icon {
  width: 48px;
  height: 48px;
  opacity: 0.5;
}

.modal-empty-title {
  font-size: var(--search-snippet-font-size-base);
  font-weight: var(--search-snippet-font-weight-medium);
  color: var(--search-snippet-text-color);
}

.modal-empty-description {
  font-size: var(--search-snippet-font-size-sm);
}

/* Footer */
.modal-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--search-snippet-spacing-sm) var(--search-snippet-spacing-md);
  border-top: var(--search-snippet-border-width) solid var(--search-snippet-border-color);
  background: var(--search-snippet-surface);
  font-size: var(--search-snippet-font-size-sm);
  color: var(--search-snippet-text-secondary);
  border-radius: 0 0 var(--search-snippet-border-radius) var(--search-snippet-border-radius);
}

.modal-footer-hints {
  display: flex;
  align-items: center;
  gap: var(--search-snippet-spacing-md);
}

.modal-footer-hint {
  display: flex;
  align-items: center;
  gap: var(--search-snippet-spacing-xs);
}

.modal-footer-hint .modal-kbd {
  min-width: 20px;
  height: 20px;
  font-size: 11px;
}

/* Results count */
.modal-results-count {
  font-size: var(--search-snippet-font-size-sm);
  color: var(--search-snippet-text-secondary);
}

/* Powered by in modal footer */
.modal-footer .powered-by-inline {
  font-size: var(--search-snippet-font-size-sm);
  color: var(--search-snippet-text-secondary);
}

.modal-footer .powered-by-inline a {
  color: var(--search-snippet-text-secondary);
  text-decoration: none;
  transition: color var(--search-snippet-transition-fast);
}

.modal-footer .powered-by-inline a:hover {
  color: var(--search-snippet-primary-color);
}

/* See more link */
.modal-see-more {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--search-snippet-spacing-xs);
  padding: var(--search-snippet-spacing-md);
  font-size: var(--search-snippet-font-size-sm);
  color: var(--search-snippet-primary-color);
  text-decoration: none;
  font-weight: var(--search-snippet-font-weight-medium);
  transition: background var(--search-snippet-transition-fast);
  padding-bottom: var(--search-snippet-spacing-xs);
}

.modal-see-more:hover {
  background: var(--search-snippet-hover);
  text-decoration: underline;
}

/* Responsive adjustments */
@media (max-width: 640px) {
  .modal-container {
    top: 10%;
    width: 95%;
    max-height: 80vh;
  }

  .modal-footer-hints {
    display: none;
  }
}

/* Animation for modal open */
@keyframes modal-slide-in {
  from {
    opacity: 0;
    transform: translateX(-50%) scale(0.95) translateY(-10px);
  }
  to {
    opacity: 1;
    transform: translateX(-50%) scale(1) translateY(0);
  }
}

.modal-container.open {
  animation: modal-slide-in var(--search-snippet-transition) ease-out;
}
`,ee="search-snippet:recent-results",T="search-snippet:favorite-results",Ce=5;function Ee(r){if(!r||typeof r!="object")return!1;const e=r;return e.type==="result"&&typeof e.id=="string"&&typeof e.title=="string"&&typeof e.description=="string"&&(e.url===void 0||typeof e.url=="string")&&(e.image===void 0||typeof e.image=="string")&&(e.timestamp===void 0||typeof e.timestamp=="number")&&(e.metadata===void 0||typeof e.metadata=="object"&&e.metadata!==null)}function x(r){return r.url||r.id}function E(r,e){const t=x(e);return r.some(s=>x(s)===t)}function S(r){try{const e=localStorage.getItem(r);if(!e)return[];const t=JSON.parse(e);return Array.isArray(t)?t.filter(Ee):[]}catch{return[]}}function Re(r,e=ee,t=Ce){const s=x(r),i=S(e).filter(a=>x(a)!==s),n=[r,...i].slice(0,Math.max(0,t));try{localStorage.setItem(e,JSON.stringify(n))}catch{}return n}function Ae(r,e=T){const t=S(e),s=E(t,r)?t.filter(i=>x(i)!==x(r)):[r,...t];try{localStorage.setItem(e,JSON.stringify(s))}catch{}return s}const j="search-modal-snippet",N=10,H=50;class Ie extends HTMLElement{constructor(){super(),o(this,"shadow"),o(this,"client",null),o(this,"stats",null),o(this,"backdrop",null),o(this,"modal",null),o(this,"inputElement",null),o(this,"resultsContainer",null),o(this,"footerCount",null),o(this,"isOpen",!1),o(this,"results",[]),o(this,"favoriteResults",[]),o(this,"activeIndex",-1),o(this,"debouncedSearch",null),o(this,"currentSearchController",null),o(this,"loadingMessageInterval",null),o(this,"loadingMessageIndex",0),o(this,"translationsOverride",null),o(this,"resolvedTranslations",f(null)),o(this,"lastSearchQuery",""),o(this,"lastSearchTotal",0),o(this,"handleGlobalKeydown",null),o(this,"handleInputChange",null),o(this,"handleInputKeydown",null),o(this,"handleBackdropClick",null),o(this,"handleResultsContainerClick",null),o(this,"savedBodyStyles",null),o(this,"savedHtmlOverflow",null),this.shadow=this.attachShadow({mode:"open"})}static get observedAttributes(){return["api-url","placeholder","max-results","max-render-results","theme","shortcut","use-meta-key","debounce-ms","hide-branding","show-url","show-date","hide-thumbnails","see-more","disable-analytics","request-options","translations"]}connectedCallback(){this.syncTranslationsFromAttribute(),this.initializeClient(),this.render(),this.attachGlobalKeyboardShortcut(),this.dispatchEvent(k("ready",void 0))}disconnectedCallback(){this.cleanup()}attributeChangedCallback(e,t,s){t!==s&&(e==="api-url"||e==="disable-analytics"?this.initializeClient():e==="theme"?this.updateTheme(s):e==="translations"&&(this.syncTranslationsFromAttribute(),this.isConnected&&this.rerender()))}get translations(){return this.translationsOverride}set translations(e){this.translationsOverride=e??null,this.resolvedTranslations=f(this.translationsOverride),this.isConnected&&this.rerender()}rerender(){const e=this.isOpen,t=this.inputElement?.value??"";this.isOpen=!1,this.render(),e&&(this.isOpen=!0,this.backdrop?.classList.add("open"),this.modal?.classList.add("open"),requestAnimationFrame(()=>{requestAnimationFrame(()=>{this.inputElement?.focus()})})),t&&this.inputElement&&(this.inputElement.value=t,t.trim().length>0&&this.performSearch(t.trim()))}syncTranslationsFromAttribute(){if(this.translationsOverride){this.resolvedTranslations=f(this.translationsOverride);return}const e=D(this.getAttribute("translations"),"SearchModalSnippet");this.resolvedTranslations=f(e)}getProps(){const e=this.resolvedTranslations;return{apiUrl:g(this.getAttribute("api-url"),""),placeholder:g(this.getAttribute("placeholder"),e.placeholder),maxResults:w(this.getAttribute("max-results"),H),maxRenderResults:w(this.getAttribute("max-render-results"),N),debounceMs:w(this.getAttribute("debounce-ms"),300),theme:g(this.getAttribute("theme"),"auto"),shortcut:g(this.getAttribute("shortcut"),"k"),useMetaKey:this.getAttribute("use-meta-key")!=="false",hideBranding:v(this.getAttribute("hide-branding"),!1),showUrl:v(this.getAttribute("show-url"),!1),showDate:v(this.getAttribute("show-date"),!1),hideThumbnails:v(this.getAttribute("hide-thumbnails"),!1),seeMore:g(this.getAttribute("see-more"),""),groupBy:g(this.getAttribute("group-by"),""),disableAnalytics:v(this.getAttribute("disable-analytics"),!1),translations:this.translationsOverride??void 0}}getRequestOptions(){const e=this.getAttribute("request-options");if(e)try{const t=JSON.parse(e);if(t===null||typeof t!="object"||Array.isArray(t))throw new Error("request-options must be a JSON object");return t}catch(t){console.error("SearchModalSnippet: invalid request-options attribute",t);return}}initializeClient(){const e=this.getProps();if(!e.apiUrl){console.error("SearchModalSnippet: api-url attribute is required"),this.client=null,this.destroyStatsClient(),this.showMissingApiUrlError();return}try{this.client=W(e.apiUrl),this.destroyStatsClient(),e.disableAnalytics||(this.stats=new X(e.apiUrl))}catch(t){console.error("SearchModalSnippet:",t)}}destroyStatsClient(){this.stats&&(this.stats.destroy(),this.stats=null)}render(){const e=this.getProps(),t=this.resolvedTranslations,s=l=>this.performSearch(l);this.debouncedSearch=G(s,e.debounceMs||300);const i=document.createElement("style");i.textContent=`${F}
${Se}`;const n=e.hideBranding?"":`<div class="powered-by-inline">${P}</div>`,a=document.createElement("div");a.innerHTML=`
      <div class="modal-backdrop" role="presentation"></div>
      <div class="modal-container" role="dialog" aria-modal="true" aria-labelledby="modal-title">
        <div class="modal-header">
          <svg class="modal-search-icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960" fill="currentColor">
            <path d="M784-120 532-372q-30 24-69 38t-83 14q-109 0-184.5-75.5T120-580q0-109 75.5-184.5T380-840q109 0 184.5 75.5T640-580q0 44-14 83t-38 69l252 252-56 56ZM380-400q75 0 127.5-52.5T560-580q0-75-52.5-127.5T380-760q-75 0-127.5 52.5T200-580q0 75 52.5 127.5T380-400Z"/>
          </svg>
          <input
            type="text"
            class="modal-search-input"
            placeholder="${c(e.placeholder||t.placeholder)}"
            aria-label="${c(t.searchButtonLabel)}"
            aria-autocomplete="list"
            aria-controls="modal-results-list"
            aria-expanded="false"
            autocomplete="off"
            spellcheck="false"
          />
        </div>
        <div class="modal-content">
          <div class="modal-results" id="modal-results-list" role="listbox" aria-label="${c(t.searchResultsAriaLabel)}"></div>
        </div>
        <div class="modal-footer">
          <div class="modal-footer-hints">
            <div class="modal-footer-hint">
              <kbd class="modal-kbd">↑</kbd>
              <kbd class="modal-kbd">↓</kbd>
              <span>${c(t.navigateHint)}</span>
            </div>
            <div class="modal-footer-hint">
              <kbd class="modal-kbd">↵</kbd>
              <span>${c(t.selectHint)}</span>
            </div>
            <div class="modal-footer-hint">
              <kbd class="modal-kbd">Esc</kbd>
              <span>${c(t.closeHint)}</span>
            </div>
          </div>
          ${n}
        </div>
      </div>
    `,this.shadow.innerHTML="",this.shadow.appendChild(i),this.shadow.appendChild(a),this.backdrop=this.shadow.querySelector(".modal-backdrop"),this.modal=this.shadow.querySelector(".modal-container"),this.inputElement=this.shadow.querySelector(".modal-search-input"),this.resultsContainer=this.shadow.querySelector(".modal-results"),this.footerCount=this.shadow.querySelector(".modal-results-count"),this.attachEventListeners(),this.client?this.showEmptyState():this.showMissingApiUrlError()}attachGlobalKeyboardShortcut(){const e=this.getProps(),t=e.shortcut?.toLowerCase()||"k";this.handleGlobalKeydown=s=>{(e.useMetaKey&&s.metaKey||s.ctrlKey)&&s.key.toLowerCase()===t&&!this.isOpen&&(s.preventDefault(),this.open())},document.addEventListener("keydown",this.handleGlobalKeydown)}attachEventListeners(){!this.inputElement||!this.backdrop||(this.handleInputChange=e=>{const t=e.target.value.trim();t.length>0&&this.debouncedSearch?this.debouncedSearch(t):(this.debouncedSearch?.cancel(),this.currentSearchController?.abort(),this.results=[],this.activeIndex=-1,this.showEmptyState())},this.inputElement.addEventListener("input",this.handleInputChange),this.handleInputKeydown=e=>{switch(e.key){case"ArrowDown":e.preventDefault(),this.navigateResults(1);break;case"ArrowUp":e.preventDefault(),this.navigateResults(-1);break;case"Enter":e.preventDefault(),this.selectActiveResult();break;case"Escape":e.preventDefault(),this.close();break}},this.inputElement.addEventListener("keydown",this.handleInputKeydown),this.handleBackdropClick=e=>{e.target===this.backdrop&&this.close()},this.backdrop.addEventListener("click",this.handleBackdropClick))}navigateResults(e){if(this.results.length===0)return;const t=this.activeIndex+e;t<0?this.activeIndex=this.results.length-1:t>=this.results.length?this.activeIndex=0:this.activeIndex=t,this.updateActiveResult()}updateActiveResult(){const e=this.resultsContainer?.querySelectorAll(".modal-result-item");e&&(e.forEach((t,s)=>{s===this.activeIndex?(t.classList.add("active"),t.setAttribute("aria-selected","true"),t.scrollIntoView({block:"nearest"})):(t.classList.remove("active"),t.setAttribute("aria-selected","false"))}),this.inputElement&&this.activeIndex>=0?this.inputElement.setAttribute("aria-activedescendant",`result-${this.activeIndex}`):this.inputElement&&this.inputElement.removeAttribute("aria-activedescendant"))}selectActiveResult(){if(this.activeIndex<0||this.activeIndex>=this.results.length){const s=this.inputElement?.value.trim();s&&s.length>0&&this.performSearch(s);return}const e=this.results[this.activeIndex];this.dispatchEvent(k("result-select",{result:e,index:this.activeIndex}));const t=this.resultsContainer?.querySelector(`.modal-result-item[data-index="${this.activeIndex}"]`);t&&I(e.url)&&t.click(),this.close()}async performSearch(e){if(!this.client){this.showMissingApiUrlError();return}this.currentSearchController&&(this.currentSearchController.abort(),this.currentSearchController=null),this.currentSearchController=new AbortController,this.showLoadingState();try{const t=this.getProps(),s=await this.client.search(e,{signal:this.currentSearchController.signal,maxResults:t.maxResults||H,request:this.getRequestOptions()}),i=s.slice(0,t.maxRenderResults||N);this.lastSearchQuery=e,this.lastSearchTotal=s.length,this.stats?.trackSearch(e,s.length),this.displayResults(i,e,s.length)}catch(t){if(t.name==="AbortError")return;this.showErrorState(t.message)}finally{this.currentSearchController=null}}displayResults(e,t,s=e.length){const i=this.getProps(),n=this.resolvedTranslations,a=i.groupBy?Y(e,i.groupBy,n.groupOther):null;if(this.results=a?a.flatMap(u=>u.results):e,this.favoriteResults=S(T),this.activeIndex=this.results.length>0?0:-1,this.clearLoadingInterval(),!this.resultsContainer)return;if(this.results.length===0){this.showNoResultsState(t);return}const l=a?Z(a,(u,m)=>this.renderResult(u,m)):this.results.map((u,m)=>this.renderResult(u,m)).join(""),h=s>e.length,d=h?y(n.resultsCountOverflow,{n:e.length,total:s}):y(s===1?n.modalResultsCount:n.modalResultsCountPlural,{n:s}),p=i.seeMore&&h?`<a href="${c(i.seeMore+encodeURIComponent(t))}" class="modal-see-more">
            <span>${c(n.seeMoreResults)}</span>
            <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14"/><path d="m12 5 7 7-7 7"/></svg>
          </a>`:"";this.resultsContainer.innerHTML=l+p,this.footerCount&&(this.footerCount.textContent=d),this.inputElement&&this.inputElement.setAttribute("aria-expanded","true"),this.attachResultHandlers(),this.updateActiveResult()}renderResult(e,t){const s=this.getProps(),i=Q(e.metadata?.icon),n=s.hideThumbnails?"":this.renderResultImage(e.image,e.title),a=I(e.url),l=a?c(a):"#",h=a?c(J(a)):"",d=s.showDate&&e.timestamp!==void 0?`<div class="modal-result-date">${c(_(e.timestamp))}</div>`:"",p=s.showUrl&&a||d?`<div class="modal-result-metadata">
            ${s.showUrl&&a?`<span class="modal-result-url">${h}</span>`:'<span class="modal-result-url modal-result-url-empty"></span>'}
            ${d}
          </div>`:"",u=E(this.favoriteResults,e),m=u?this.resolvedTranslations.removeFavorite:this.resolvedTranslations.addFavorite;return`
      <div class="modal-result-row">
        <a
          href="${l}"
          class="modal-result-item${t===this.activeIndex?" active":""}"
          role="option"
          id="result-${t}"
          aria-selected="${t===this.activeIndex}"
          tabindex="-1"
          data-index="${t}"
          data-result-id="${c(e.id||"")}"
          data-url="${c(a)}"
        >
          ${i}
          ${n}
          <div class="modal-result-content">
            <div class="modal-result-title">${c(e.title||"")}</div>
            ${e.description?`<div class="modal-result-description">${c(e.description)}</div>`:""}
            ${p}
          </div>
        </a>
        <button
          type="button"
          class="modal-favorite-button${u?" favorite":""}"
          data-favorite-index="${t}"
          aria-label="${c(m)}"
          title="${c(m)}"
          aria-pressed="${u}"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="m12 2.7 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2-5.6-3-5.6 3 1.1-6.2L3 9.3l6.2-.9L12 2.7Z"></path>
          </svg>
        </button>
      </div>
    `}renderResultImage(e,t){const s='<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="16" y1="13" x2="8" y2="13"/><line x1="16" y1="17" x2="8" y2="17"/><polyline points="10 9 9 9 8 9"/></svg>';return e?`
      <div class="modal-result-image-container">
        <div class="modal-result-image-loading"></div>
        <div class="modal-result-image-placeholder" style="display: none;">${s}</div>
        <img 
          class="modal-result-image" 
          src="${c(e)}" 
          alt="${c(t)}"
          loading="lazy"
        />
      </div>
    `:`
        <div class="modal-result-image-container">
          <div class="modal-result-image-placeholder">${s}</div>
        </div>
      `}attachResultHandlers(){this.detachResultsContainerClick();const e=this.resultsContainer;e&&(this.handleResultsContainerClick=t=>{const s=t.target;if(!s)return;const i=s.closest(".modal-favorite-button");if(i){const a=Number.parseInt(i.dataset.favoriteIndex??"",10),l=this.results[a];if(!Number.isNaN(a)&&l)if(this.favoriteResults=Ae(l),this.inputElement?.value.trim()){const h=E(this.favoriteResults,l),d=h?this.resolvedTranslations.removeFavorite:this.resolvedTranslations.addFavorite;i.classList.toggle("favorite",h),i.setAttribute("aria-pressed",String(h)),i.setAttribute("aria-label",d),i.title=d}else this.showEmptyState();return}const n=s.closest(".modal-result-item");if(n){n.getAttribute("href")==="#"&&t.preventDefault();const a=n.getAttribute("data-index"),l=n.getAttribute("data-result-id")??"",h=a!==null?Number.parseInt(a,10):Number.NaN;if(!Number.isNaN(h)){const d=this.results[h];d&&Re(d),l&&this.stats?.trackClick(this.lastSearchQuery,this.lastSearchTotal,l,h)}return}s.closest(".modal-see-more")&&this.stats?.trackViewMore(this.lastSearchQuery,this.lastSearchTotal)},e.addEventListener("click",this.handleResultsContainerClick),e.querySelectorAll(".modal-result-item").forEach((t,s)=>{t.addEventListener("mouseenter",()=>{this.activeIndex=s,this.updateActiveResult()})}),e.querySelectorAll(".modal-result-image").forEach(t=>{t.addEventListener("load",()=>{t.classList.add("loaded"),t.closest(".modal-result-image-container")?.querySelector(".modal-result-image-loading")?.remove()}),t.addEventListener("error",()=>{const s=t.closest(".modal-result-image-container");s?.querySelector(".modal-result-image-loading")?.remove();const i=s?.querySelector(".modal-result-image-placeholder");i&&(i.style.display="flex"),t.style.display="none"})}))}detachResultsContainerClick(){this.resultsContainer&&this.handleResultsContainerClick&&this.resultsContainer.removeEventListener("click",this.handleResultsContainerClick),this.handleResultsContainerClick=null}renderEmptyState(){const e=this.resolvedTranslations;return`
      <div class="modal-empty">
        <svg class="modal-empty-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="11" cy="11" r="8"></circle>
          <path d="m21 21-4.35-4.35"></path>
        </svg>
        <div class="modal-empty-description">${c(e.modalEmptyStateDescription)}</div>
      </div>
    `}renderInitialSection(e,t,s,i,n){return e.length===0?"":`
      <section class="modal-initial-section" aria-labelledby="${s}">
        <h2 class="modal-initial-section-title" id="${s}">
          ${n==="favorite"?`<svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="m12 2.7 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2-5.6-3-5.6 3 1.1-6.2L3 9.3l6.2-.9L12 2.7Z"></path>
          </svg>`:`<svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M3 12a9 9 0 1 0 3-6.7L3 8"></path>
            <path d="M3 3v5h5"></path>
            <path d="M12 7v5l4 2"></path>
          </svg>`}
          <span>${c(t)}</span>
        </h2>
        ${e.map((a,l)=>this.renderResult(a,i+l)).join("")}
      </section>
    `}showEmptyState(){if(this.clearLoadingInterval(),!this.resultsContainer)return;this.favoriteResults=S(T);const e=S(ee).filter(t=>!E(this.favoriteResults,t));this.results=[...this.favoriteResults,...e],this.activeIndex=this.results.length>0?0:-1,this.resultsContainer.innerHTML=this.results.length>0?`${this.renderInitialSection(this.favoriteResults,this.resolvedTranslations.favoriteResults,"modal-favorite-results-title",0,"favorite")}${this.renderInitialSection(e,this.resolvedTranslations.recentResults,"modal-recent-results-title",this.favoriteResults.length,"recent")}`:this.renderEmptyState(),this.attachResultHandlers(),this.updateActiveResult(),this.footerCount&&(this.footerCount.textContent=""),this.inputElement&&this.inputElement.setAttribute("aria-expanded",String(this.results.length>0))}showLoadingState(){if(!this.resultsContainer)return;this.clearLoadingInterval();const e=this.resolvedTranslations.loadingMessages,t=this.resolvedTranslations;this.loadingMessageIndex=Math.floor(Math.random()*e.length),this.resultsContainer.innerHTML=`
      <div class="modal-loading">
        <div class="loading" aria-label="${c(t.loadingAriaLabel)}"></div>
        <div class="loading-text loading-text-animate">${c(e[this.loadingMessageIndex])}</div>
      </div>
    `,this.footerCount&&(this.footerCount.textContent=e[this.loadingMessageIndex]),this.startLoadingInterval()}startLoadingInterval(){this.loadingMessageInterval=setInterval(()=>{const e=this.resolvedTranslations.loadingMessages;this.loadingMessageIndex=(this.loadingMessageIndex+1)%e.length;const t=this.resultsContainer?.querySelector(".loading-text");t&&(t.classList.remove("loading-text-animate"),t.offsetWidth,t.textContent=e[this.loadingMessageIndex],t.classList.add("loading-text-animate")),this.footerCount&&(this.footerCount.textContent=e[this.loadingMessageIndex])},V)}clearLoadingInterval(){this.loadingMessageInterval&&(clearInterval(this.loadingMessageInterval),this.loadingMessageInterval=null)}showNoResultsState(e){if(this.clearLoadingInterval(),!this.resultsContainer)return;const t=this.resolvedTranslations;this.resultsContainer.innerHTML=`
      <div class="modal-empty">
        <svg class="modal-empty-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="11" cy="11" r="8"></circle>
          <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
        </svg>
        <div class="modal-empty-title">${c(t.modalNoResultsTitle)}</div>
        <div class="modal-empty-description">${c(y(t.modalNoResultsDescription,{query:e}))}</div>
      </div>
    `,this.footerCount&&(this.footerCount.textContent=t.modalResultsCountZero),this.inputElement&&this.inputElement.setAttribute("aria-expanded","false")}showErrorState(e){if(this.clearLoadingInterval(),!this.resultsContainer)return;const t=this.resolvedTranslations;this.resultsContainer.innerHTML=`
      <div class="error">
        <strong>${c(t.errorPrefix)}</strong> ${c(e)}
      </div>
    `,this.footerCount&&(this.footerCount.textContent=t.modalResultsCountError)}showMissingApiUrlError(){this.resultsContainer&&this.showErrorState(this.resolvedTranslations.missingApiUrlError)}updateTheme(e){const t=e==="light"||e==="dark"||e==="auto"?e:"auto";t==="auto"?this.removeAttribute("theme"):this.setAttribute("theme",t)}lockBodyScroll(){const e=window.scrollY;this.savedBodyStyles={overflow:document.body.style.overflow,position:document.body.style.position,top:document.body.style.top,width:document.body.style.width,scrollbarGutter:document.body.style.scrollbarGutter},this.savedHtmlOverflow=document.documentElement.style.overflow,document.body.style.scrollbarGutter="stable",document.documentElement.style.overflow="hidden",document.body.style.overflow="hidden",document.body.style.position="fixed",document.body.style.top=`-${e}px`,document.body.style.width="100%"}unlockBodyScroll(){if(!this.savedBodyStyles)return;const e=Math.abs(Number.parseInt(document.body.style.top||"0",10));document.documentElement.style.overflow=this.savedHtmlOverflow||"",document.body.style.overflow=this.savedBodyStyles.overflow,document.body.style.position=this.savedBodyStyles.position,document.body.style.top=this.savedBodyStyles.top,document.body.style.width=this.savedBodyStyles.width,document.body.style.scrollbarGutter=this.savedBodyStyles.scrollbarGutter||"",window.scrollTo(0,e),this.savedBodyStyles=null,this.savedHtmlOverflow=null}cleanup(){this.clearLoadingInterval(),this.currentSearchController&&(this.currentSearchController.abort(),this.currentSearchController=null),this.handleGlobalKeydown&&(document.removeEventListener("keydown",this.handleGlobalKeydown),this.handleGlobalKeydown=null),this.inputElement&&(this.handleInputChange&&this.inputElement.removeEventListener("input",this.handleInputChange),this.handleInputKeydown&&this.inputElement.removeEventListener("keydown",this.handleInputKeydown)),this.backdrop&&this.handleBackdropClick&&this.backdrop.removeEventListener("click",this.handleBackdropClick),this.detachResultsContainerClick(),this.handleInputChange=null,this.handleInputKeydown=null,this.handleBackdropClick=null,this.destroyStatsClient(),this.client&&this.client.cancelAllRequests()}open(){this.isOpen||(this.isOpen=!0,this.backdrop?.classList.add("open"),this.modal?.classList.add("open"),requestAnimationFrame(()=>{requestAnimationFrame(()=>{this.inputElement?.focus()})}),this.lockBodyScroll(),this.dispatchEvent(k("open",void 0)))}close(){this.isOpen&&(this.isOpen=!1,this.backdrop?.classList.remove("open"),this.modal?.classList.remove("open"),this.inputElement&&(this.inputElement.value=""),this.results=[],this.activeIndex=-1,this.showEmptyState(),this.unlockBodyScroll(),this.dispatchEvent(k("close",void 0)))}toggle(){this.isOpen?this.close():this.open()}async search(e){this.isOpen||this.open(),this.inputElement&&(this.inputElement.value=e),await this.performSearch(e)}getResults(){return[...this.results]}isModalOpen(){return this.isOpen}}customElements.get(j)||customElements.define(j,Ie);const Te="[data-ai-search-trigger]",R=()=>document.querySelector("search-modal-snippet");let b=null,L=0;const U=()=>{b!==null&&window.cancelAnimationFrame(b);const r=++L;b=window.requestAnimationFrame(()=>{b=null;const t=R()?.shadowRoot?.querySelector("input"),i=t?.closest('[role="dialog"]')?.getAnimations()??[];Promise.allSettled(i.map(({finished:n})=>n)).then(()=>{r!==L||!R()?.isModalOpen()||t?.focus()})})},Le=r=>{const e=r.target;!(e instanceof Element)||!e.closest(Te)||R()?.open()},te=()=>{const r=R();r?.removeEventListener("open",U),r?.addEventListener("open",U)},Me=()=>{L++,b!==null&&window.cancelAnimationFrame(b),b=null};te();document.addEventListener("click",Le);document.addEventListener("astro:page-load",te);document.addEventListener("astro:before-swap",Me);
