//! Ses durumu: kim hangi ses kanalında, kim konuşuyor ve kendi ses bağlantımız.

use std::collections::{HashMap, HashSet};

use digcord_model::{Guild, Snowflake, VoiceState};

use crate::State;

/// Kendi ses bağlantımızın aşaması.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoicePhase {
    /// op 4 gönderildi; ana gateway'den oturum ve sunucu bilgisi bekleniyor.
    Joining,
    /// Ses sunucusuna bağlanılıyor.
    Connecting,
    /// Ses akıyor. `e2ee`: DAVE etkin.
    Connected { e2ee: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceCall {
    pub guild: Option<Snowflake>,
    pub channel: Snowflake,
    pub phase: VoicePhase,
    pub self_mute: bool,
    pub self_deaf: bool,
}

#[derive(Default)]
pub(crate) struct VoiceData {
    /// (guild; DM için `None`, kullanıcı) → durum.
    states: HashMap<(Option<Snowflake>, Snowflake), VoiceState>,
    speaking: HashSet<Snowflake>,
    call: Option<VoiceCall>,
}

impl State {
    pub(crate) fn load_guild_voice(&mut self, guild: &Guild) {
        self.voice.states.retain(|(g, _), _| *g != Some(guild.id));
        for vs in &guild.voice_states {
            if vs.channel_id.is_some() {
                let mut vs = vs.clone();
                vs.guild_id = Some(guild.id);
                self.voice.states.insert((Some(guild.id), vs.user_id), vs);
            }
        }
    }

    /// `VOICE_STATE_UPDATE`: kanal `None` ise kullanıcı ayrıldı.
    pub fn voice_state_update(&mut self, vs: &VoiceState) {
        self.version += 1;
        let key = (vs.guild_id, vs.user_id);
        if vs.channel_id.is_some() {
            self.voice.states.insert(key, vs.clone());
        } else {
            self.voice.states.remove(&key);
            self.voice.speaking.remove(&vs.user_id);
        }
    }

    /// Ses kanalındaki kullanıcılar (id sırasıyla).
    pub fn voice_members(&self, channel: Snowflake) -> Vec<&VoiceState> {
        let mut v: Vec<&VoiceState> = self
            .voice
            .states
            .values()
            .filter(|s| s.channel_id == Some(channel))
            .collect();
        v.sort_by_key(|s| s.user_id);
        v
    }

    /// Ana gateway'in verdiği kendi ses oturum kimliğimiz (o guild'de / DM'de bağlıysak).
    pub fn my_voice_session(&self, guild: Option<Snowflake>) -> Option<&str> {
        let me = self.me.as_ref()?.id;
        self.voice
            .states
            .get(&(guild, me))
            .map(|s| s.session_id.as_str())
            .filter(|s| !s.is_empty())
    }

    pub fn set_speaking(&mut self, user: Snowflake, speaking: bool) {
        let changed = if speaking {
            self.voice.speaking.insert(user)
        } else {
            self.voice.speaking.remove(&user)
        };
        if changed {
            self.version += 1;
        }
    }

    pub fn is_speaking(&self, user: Snowflake) -> bool {
        self.voice.speaking.contains(&user)
    }

    pub fn voice_call(&self) -> Option<&VoiceCall> {
        self.voice.call.as_ref()
    }

    pub fn set_voice_call(&mut self, call: Option<VoiceCall>) {
        self.version += 1;
        if call.is_none() {
            self.voice.speaking.clear();
        }
        self.voice.call = call;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use digcord_model::Ready;

    fn vs(guild: Option<u64>, channel: Option<u64>, user: u64) -> VoiceState {
        serde_json::from_value(serde_json::json!({
            "guild_id": guild.map(|g| g.to_string()),
            "channel_id": channel.map(|c| c.to_string()),
            "user_id": user.to_string(), "session_id": format!("s{user}"),
            "self_mute": user == 3
        }))
        .expect("ses durumu")
    }

    fn state() -> State {
        let ready: Ready = serde_json::from_value(serde_json::json!({
            "v": 9, "user": {"id": "1", "username": "ben"}, "session_id": "s",
            "resume_gateway_url": "wss://x",
            "guilds": [{"id": "10", "properties": {"name": "G"},
                "channels": [{"id": "20", "type": 2, "name": "Ses"}],
                "voice_states": [{"channel_id": "20", "user_id": "2", "session_id": "x"}]}],
            "private_channels": [], "users": []
        }))
        .expect("ready");
        let mut st = State::new();
        st.load_ready(&ready);
        st
    }

    #[test]
    fn ready_and_updates() {
        let mut st = state();
        let ids = |st: &State| -> Vec<u64> {
            st.voice_members(Snowflake(20))
                .iter()
                .map(|s| s.user_id.0)
                .collect()
        };
        assert_eq!(ids(&st), [2], "READY'deki ses durumu guild id'siyle");
        st.voice_state_update(&vs(Some(10), Some(20), 3));
        st.voice_state_update(&vs(Some(10), Some(20), 1));
        assert_eq!(ids(&st), [1, 2, 3]);
        assert_eq!(st.my_voice_session(Some(Snowflake(10))), Some("s1"));
        assert_eq!(st.my_voice_session(None), None);
        st.set_speaking(Snowflake(3), true);
        assert!(st.is_speaking(Snowflake(3)));
        st.voice_state_update(&vs(Some(10), None, 3));
        assert_eq!(ids(&st), [1, 2]);
        assert!(!st.is_speaking(Snowflake(3)), "ayrılan konuşmuyor");
        // DM araması ayrı anahtar.
        st.voice_state_update(&vs(None, Some(50), 1));
        assert_eq!(st.my_voice_session(None), Some("s1"));
        assert_eq!(st.voice_members(Snowflake(50)).len(), 1);
    }

    #[test]
    fn call_state() {
        let mut st = state();
        st.set_voice_call(Some(VoiceCall {
            guild: Some(Snowflake(10)),
            channel: Snowflake(20),
            phase: VoicePhase::Joining,
            self_mute: false,
            self_deaf: false,
        }));
        st.set_speaking(Snowflake(2), true);
        assert_eq!(st.voice_call().map(|c| c.phase), Some(VoicePhase::Joining));
        st.set_voice_call(None);
        assert!(!st.is_speaking(Snowflake(2)));
    }
}
