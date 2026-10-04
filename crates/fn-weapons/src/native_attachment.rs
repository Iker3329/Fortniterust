//! Observed control flow of FortPawn's equipped-weapon attachment update,
//! original RVA 0x802D30. Evidence: pawn-weapon-attachment-body.json.
//! Unknown callback semantics remain explicit; this is not a render transform.

pub trait AttachmentBackend {
    type Weapon: Copy + Eq;
    type Mesh: Copy;
    type Socket: Copy;
    fn attached_940(&self) -> Option<Self::Weapon>;
    fn current_830(&self) -> Option<Self::Weapon>;
    fn set_attached_940(&mut self, weapon: Option<Self::Weapon>);
    fn mesh_3e8(&self) -> Self::Mesh;
    fn socket_850(&self) -> Self::Socket;
    fn flags_84(&self) -> u8;
    fn byte_b80(&self) -> u8;
    fn call_931ee0(&mut self, weapon: Self::Weapon);
    fn call_93aff0(&mut self, weapon: Option<Self::Weapon>, first: bool, second: bool);
    fn detach_virtual_758(&mut self, weapon: Option<Self::Weapon>);
    fn attach_virtual_750(&mut self, weapon: Self::Weapon, mesh: Self::Mesh, socket: Self::Socket);
    fn call_931e30(&mut self, current: Option<Self::Weapon>, previous: Option<Self::Weapon>);
    fn event_a3f8b0(&mut self, current: Option<Self::Weapon>, previous: Option<Self::Weapon>);
    fn delegate_27f3b0(&mut self, current: Option<Self::Weapon>, previous: Option<Self::Weapon>);
}

/// Preserve the original loads AFTER callbacks. A callback can change current
/// or attached; replacing these loads with the initial snapshot is incorrect.
/// Nullable arguments reflect the native unchecked loads, not invented gates.
pub fn update_attachment<B: AttachmentBackend>(b: &mut B) {
    let previous = b.attached_940();
    if let Some(old) = previous {
        if b.current_830() == previous { return; }
        b.call_931ee0(old);
        let attached = b.attached_940();
        b.call_93aff0(attached, false, true);
        let attached = b.attached_940();
        b.detach_virtual_758(attached);
        b.set_attached_940(None);
    }
    let Some(current) = b.current_830() else { return; };
    b.attach_virtual_750(current, b.mesh_3e8(), b.socket_850());
    let first = b.flags_84() & 1 == 0 && b.byte_b80() == 0;
    let current = b.current_830();
    b.set_attached_940(current);
    b.call_93aff0(current, first, true);
    let current = b.current_830();
    b.call_931e30(current, previous);
    let current = b.current_830();
    b.event_a3f8b0(current, previous);
    let current = b.current_830();
    b.delegate_27f3b0(current, previous);
}

/// File-backed rule bytes passed to the actor-root attachment helper. Keep
/// them raw until the original component transform behavior is fully ported.
pub const ROOT_RULE_BYTES: [u8; 4] = [2, 2, 2, 0];

/// AFortWeapon attach virtual +0x750, RVA 0x918860. The three components and
/// owner are loaded after earlier callbacks, not captured before root attach.
/// The component rule object is identified by its original VA, NOT assigned
/// invented bytes: its startup value still requires independent evidence.
pub trait WeaponAttachmentBackend {
    type Mesh: Copy;
    type Socket: Copy;
    type Component: Copy;
    type Owner: Copy;
    type AttributeObject: Copy;
    fn attach_actor_root_1f2b730(&mut self, parent: Self::Mesh, socket: Self::Socket, rules: [u8;4]);
    fn component_at(&self, offset: usize) -> Option<Self::Component>;
    fn attach_component_20cb370(&mut self, component: Option<Self::Component>, parent: Self::Mesh, socket: Self::Socket, rules_va: u64);
    fn owner_150(&self) -> Option<Self::Owner>;
    fn owner_matches_a05c20(&mut self, owner: Self::Owner) -> bool;
    fn owner_interface_7c0_virtual_10(&mut self, owner: Self::Owner) -> Option<Self::AttributeObject>;
    /// Includes original attribute-identity initialization, query 0x3E3B950
    /// and handle cleanup 0x1281200. No inferred attribute name or unit.
    fn query_original_scalar(&mut self, object: Self::AttributeObject) -> f32;
    fn write_scalar_80(&mut self, value: f32);
    fn attached_event_a3f860(&mut self);
}

pub fn attach_weapon<B: WeaponAttachmentBackend>(b: &mut B, parent: B::Mesh, socket: B::Socket) {
    b.attach_actor_root_1f2b730(parent, socket, ROOT_RULE_BYTES);
    for offset in [0x8f8, 0x900, 0x908] {
        let component = b.component_at(offset);
        // Native caller has no null gate here. Do not skip a callback because
        // an intuitive renderer implementation might skip missing components.
        b.attach_component_20cb370(component, parent, socket, 0x14685a9bc);
    }
    if let Some(owner) = b.owner_150() {
        if b.owner_matches_a05c20(owner) {
            if let Some(object) = b.owner_interface_7c0_virtual_10(owner) {
                let value = b.query_original_scalar(object);
                b.write_scalar_80(value);
            }
        }
    }
    b.attached_event_a3f860();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Debug, PartialEq)]
    enum Call { Before(u32), Flag(Option<u32>, bool, bool), Detach(Option<u32>), Set(Option<u32>), Attach(u32,u32,u64), After(Option<u32>,Option<u32>), Event(Option<u32>,Option<u32>), Delegate(Option<u32>,Option<u32>) }
    struct Probe { attached: Option<u32>, current: Option<u32>, flags: u8, byte: u8, mutate: bool, calls: Vec<Call> }
    impl Default for Probe { fn default()->Self { Self { attached:None,current:Some(2),flags:0,byte:0,mutate:false,calls:vec![] } } }
    impl AttachmentBackend for Probe {
        type Weapon=u32; type Mesh=u32; type Socket=u64;
        fn attached_940(&self)->Option<u32>{self.attached}
        fn current_830(&self)->Option<u32>{self.current}
        fn set_attached_940(&mut self,w:Option<u32>){self.calls.push(Call::Set(w));self.attached=w;}
        fn mesh_3e8(&self)->u32{30}
        fn socket_850(&self)->u64{850}
        fn flags_84(&self)->u8{self.flags}
        fn byte_b80(&self)->u8{self.byte}
        fn call_931ee0(&mut self,w:u32){self.calls.push(Call::Before(w));if self.mutate{self.attached=Some(10);}}
        fn call_93aff0(&mut self,w:Option<u32>,a:bool,c:bool){self.calls.push(Call::Flag(w,a,c));if self.mutate {if w==Some(10){self.attached=Some(11);}else{self.current=Some(21);}}}
        fn detach_virtual_758(&mut self,w:Option<u32>){self.calls.push(Call::Detach(w));}
        fn attach_virtual_750(&mut self,w:u32,m:u32,s:u64){self.calls.push(Call::Attach(w,m,s));if self.mutate{self.current=Some(20);}}
        fn call_931e30(&mut self,w:Option<u32>,p:Option<u32>){self.calls.push(Call::After(w,p));if self.mutate{self.current=Some(22);}}
        fn event_a3f8b0(&mut self,w:Option<u32>,p:Option<u32>){self.calls.push(Call::Event(w,p));if self.mutate{self.current=Some(23);}}
        fn delegate_27f3b0(&mut self,w:Option<u32>,p:Option<u32>){self.calls.push(Call::Delegate(w,p));}
    }
    #[test] fn unchanged_non_null_weapon_returns_without_callbacks() {
        let mut p=Probe{attached:Some(2),..Probe::default()};update_attachment(&mut p);assert!(p.calls.is_empty());
    }
    #[test] fn no_current_weapon_only_detaches_old() {
        let mut p=Probe{attached:Some(1),current:None,..Probe::default()};update_attachment(&mut p);
        assert_eq!(p.calls,[Call::Before(1),Call::Flag(Some(1),false,true),Call::Detach(Some(1)),Call::Set(None)]);
        let mut p=Probe{current:None,..Probe::default()};update_attachment(&mut p);assert!(p.calls.is_empty());
    }
    #[test] fn switch_preserves_original_order_and_previous_argument() {
        let mut p=Probe{attached:Some(1),..Probe::default()};update_attachment(&mut p);
        assert_eq!(p.calls,[Call::Before(1),Call::Flag(Some(1),false,true),Call::Detach(Some(1)),Call::Set(None),Call::Attach(2,30,850),Call::Set(Some(2)),Call::Flag(Some(2),true,true),Call::After(Some(2),Some(1)),Call::Event(Some(2),Some(1)),Call::Delegate(Some(2),Some(1))]);
    }
    #[test] fn flag_gate_uses_only_low_bit_and_full_byte() {
        for flags in [0,1,2,3,128,255] {for byte in [0,1,128,255] {
            let mut p=Probe{flags,byte,..Probe::default()};update_attachment(&mut p);
            assert!(p.calls.contains(&Call::Flag(Some(2),flags&1==0&&byte==0,true)));
            assert!(p.calls.contains(&Call::After(Some(2),None)));
        }}
    }
    #[test] fn reentrant_callbacks_do_not_leave_stale_weapon_loads() {
        let mut p=Probe{attached:Some(1),mutate:true,..Probe::default()};update_attachment(&mut p);
        assert_eq!(p.calls,[Call::Before(1),Call::Flag(Some(10),false,true),Call::Detach(Some(11)),Call::Set(None),Call::Attach(2,30,850),Call::Set(Some(20)),Call::Flag(Some(20),true,true),Call::After(Some(21),Some(1)),Call::Event(Some(22),Some(1)),Call::Delegate(Some(23),Some(1))]);
        assert_eq!(p.attached,Some(20));
    }
    struct WeaponProbe { components: [Option<u32>;3], owner: Option<u32>, matches: bool, object: Option<u32>, value: Option<f32>, mutate: bool, calls: Vec<String> }
    impl Default for WeaponProbe { fn default()->Self { Self { components:[Some(1),None,Some(3)],owner:Some(4),matches:true,object:Some(5),value:None,mutate:true,calls:vec![] } } }
    impl WeaponAttachmentBackend for WeaponProbe {
        type Mesh=u32;type Socket=u64;type Component=u32;type Owner=u32;type AttributeObject=u32;
        fn attach_actor_root_1f2b730(&mut self,m:u32,s:u64,r:[u8;4]) {assert_eq!((m,s,r),(30,850,[2,2,2,0]));self.calls.push("root".into());if self.mutate{self.components[0]=Some(10);}}
        fn component_at(&self,o:usize)->Option<u32>{self.components[(o-0x8f8)/8]}
        fn attach_component_20cb370(&mut self,c:Option<u32>,m:u32,s:u64,r:u64){assert_eq!((m,s,r),(30,850,0x14685a9bc));self.calls.push(format!("component:{c:?}"));if self.mutate{if c==Some(10){self.components[1]=Some(20);}if c==Some(20){self.owner=Some(40);}}}
        fn owner_150(&self)->Option<u32>{self.owner}
        fn owner_matches_a05c20(&mut self,o:u32)->bool{self.calls.push(format!("class:{o}"));self.matches}
        fn owner_interface_7c0_virtual_10(&mut self,o:u32)->Option<u32>{self.calls.push(format!("interface:{o}"));self.object}
        fn query_original_scalar(&mut self,o:u32)->f32{self.calls.push(format!("query:{o}"));-0.0}
        fn write_scalar_80(&mut self,v:f32){self.calls.push("write".into());self.value=Some(v);}
        fn attached_event_a3f860(&mut self){self.calls.push("event".into());}
    }
    #[test] fn weapon_attachment_reloads_components_and_owner_after_callbacks() {
        let mut p=WeaponProbe::default();attach_weapon(&mut p,30,850);
        assert_eq!(p.calls,["root","component:Some(10)","component:Some(20)","component:Some(3)","class:40","interface:40","query:5","write","event"]);
        assert_eq!(p.value.unwrap().to_bits(),(-0.0f32).to_bits());
    }
    #[test] fn owner_gates_do_not_suppress_attachment_event() {
        for (owner,matches,object,tail) in [
            (None,true,Some(5),vec!["event"]),
            (Some(4),false,Some(5),vec!["class:4","event"]),
            (Some(4),true,None,vec!["class:4","interface:4","event"]),
        ] {let mut p=WeaponProbe{owner,matches,object,mutate:false,..WeaponProbe::default()};
            attach_weapon(&mut p,30,850);
            assert_eq!(&p.calls[..4],["root","component:Some(1)","component:None","component:Some(3)"]);
            assert_eq!(&p.calls[4..],tail);assert!(p.value.is_none());
        }
    }
}
