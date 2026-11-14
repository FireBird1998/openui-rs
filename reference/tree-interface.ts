// Run from reference/: bun --no-install tree-interface.ts > ../conformance/golden/runtime-interface.json
process.env.OPENUI_TELEMETRY_DISABLED='1';process.env.DO_NOT_TRACK='1';
const {compileSchema,parse}=await import('./lang-core/src/parser/parser');
const {evaluateRoot}=await import('./lang-core/src/runtime/evaluate-tree');
const {createStore}=await import('./lang-core/src/runtime/store');
const library=compileSchema({$defs:{Box:{properties:{a:{},b:{},c:{},d:{}}}}});
const rows=[];
function add(name,program,saved={}) {
 const p=parse(program,library),store=createStore();store.initialize(p.stateDeclarations??{},saved);
 const errors=[];const root=evaluateRoot(p,{ctx:{getState:n=>store.get(n),resolveRef:()=>null},library:{components:{}},store,errors});
 rows.push({name,program,saved,expected:{root,errors}});
}
for(const [terms,width,spaces,distinct]of [[10,5000,true,true],[20,2000,true,true],[40,1000,true,true],[150,40,false,false],[200,1,false,false]]) {
 const expr=Array.from({length:terms},(_,i)=>'$s'+(distinct?i:0)).join(spaces?'+" "+':'+');
 const names=Array.from({length:distinct?terms:1},(_,i)=>'$s'+i);
 add(`concat-${terms}-${width}`,`root=Box(${expr})\n`+names.map(n=>n+'=""').join('\n'),Object.fromEntries(names.map(n=>[n,'x'.repeat(width)])));
}
const bad={toString:null}, saved={$bad:bad,$obj:{a:1},$arr:[7],$key:bad,$nested:[[bad]],$rows:[{a:bad},{a:'ok'}]};
for(const [name,expr]of Object.entries({
 plus:'""+$bad',equal:'$bad=="x"',notEqual:'$bad!="x"',booleanEqual:'$bad==true',nested:'""+$nested',
 sortField:'@Sort([1,2],$bad)',sortDirection:'@Sort([1,2],"",$bad)',sortValues:'@Sort($rows,"a")',
 filterField:'@Filter([1],$bad)',filterOperator:'@Filter([1],"",$bad)',filterContains:'@Filter($rows,"a","contains","x")',filterNeedle:'@Filter(["x"],"","contains",$bad)',
 filterEqual:'@Filter($rows,"a","==","x")',filterNotEqual:'@Filter($rows,"a","!=","x")',index:'$obj[$key]',arrayIndex:'$arr[$key]',
 shortAnd:'false&&(""+$bad)',shortOr:'true||(""+$bad)',ternary:'false?(""+$bad):7',order:'[""+$bad,{k:"Null"},"after"]',
}))add('throw-'+name,`root=Box(${expr},7)\n`+Object.keys(saved).map(n=>n+'=null').join('\n'),saved);
const malformed=[
 ['array-like-args','{k:"Comp",name:"Count",args:{length:1,"0":{k:"Arr",els:[{k:"Num",v:1},{k:"Num",v:2}]}}}'],
 ['member-number','{k:"Member",obj:{k:"StateRef",n:"$o"},field:5}'],
 ['count-missing','{k:"Comp",name:"Count"}'],['count-null','{k:"Comp",name:"Count",args:null}'],['count-number','{k:"Comp",name:"Count",args:3}'],
 ['arr-missing','{k:"Arr"}'],['arr-null','{k:"Arr",els:null}'],['arr-number','{k:"Arr",els:3}'],['arr-string','{k:"Arr",els:"ab"}'],
 ['obj-missing','{k:"Obj"}'],['obj-null','{k:"Obj",entries:null}'],['obj-number','{k:"Obj",entries:3}'],
 ['entry-number','{k:"Obj",entries:[3]}'],['entry-null','{k:"Obj",entries:[null]}'],['entry-object','{k:"Obj",entries:[{}]}'],['entry-boolean','{k:"Obj",entries:[true]}'],['entry-string','{k:"Obj",entries:["ab"]}'],
 ['entry-short','{k:"Obj",entries:[[]]}'],['child-missing','{k:"BinOp",op:"+",left:{k:"Num",v:1}}'],['child-null','{k:"BinOp",op:"+",left:null,right:{k:"Num",v:1}}'],
 ['lazy-malformed','{k:"BinOp",op:"&&",left:{k:"Bool",v:false},right:{k:"Comp",name:"Count"}}'],
];
for(const[name,value]of malformed)add('descriptor-'+name,`root=Box(${value},$n)\n$n=1\n$o={a:1,"5":2}`);
for (const value of [
 '{k:"StateRef",n:["$x"]}', '{k:"StateRef",n:{toString:1}}',
 '{k:"Ref",n:{toString:1}}', '{k:"Comp",name:["Count"],args:[{k:"Arr",els:[{k:"Num",v:1}]}]}'
]) add('strict-name-'+rows.length,`root=Box(${value})\n$x=7`);
for (const name of ['descriptor-names-types','descriptor-fields-types','descriptor-state-names']) add(name,(await import('fs')).readFileSync('../conformance/cases/runtime/'+name+'.oui','utf8'));
add('each-keeps-unhandled-js-values', 'root=Box(@Each([1],x,[$o.missing])[0][0],@Each([1],x,{toString:$o.missing})[0]+"",@Each([1],x,[$o.missing]),@Each([1],x,{type:"element",typeName:"Box",props:{a:[$o.missing]},partial:false,hasDynamicProps:false})[0].props.a[0])\n$o={}');
process.stdout.write(JSON.stringify(rows)+'\n');
