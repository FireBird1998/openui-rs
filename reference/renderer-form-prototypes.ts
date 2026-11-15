// Direct form observation through the attributed renderer transcription.
import {writeFileSync} from 'fs';
import {createStore} from './lang-core/src/runtime/store';
import {rendererState,persistDefaultValue} from './renderer-actions';
const store=createStore();store.set('form',Object.create(null));
const api=rendererState({store,evaluationContext:{},schema:{},resultRef:{current:{}},queryManager:{},propsRef:{current:{}}});
const protoBefore=api.getFieldValue('form','__proto__')===undefined;
for(const [name,defaultValue] of [['constructor',1],['toString',2]])persistDefaultValue({isStreaming:false,existingValue:api.getFieldValue('form',name),defaultValue,formName:'form',name},api.setFieldValue);
const form=store.get('form');
const observation={protoBefore,constructorWritten:Object.hasOwn(form,'constructor'),toStringWritten:Object.hasOwn(form,'toString'),snapshot:store.getSnapshot(),protoAfter:api.getFieldValue('form','__proto__')};
writeFileSync(new URL('../conformance/golden/runtime/form-prototypes.json',import.meta.url),JSON.stringify(observation)+'\n');store.dispose();
