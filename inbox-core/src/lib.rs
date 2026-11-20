// run-inbox.sh の ack / hold / reply の判定だけを取り出した純粋関数。
// ファイルの移動・送信・ヘッダの読み取りは外殻 (run-inbox.sh) が行い、その結果を Obs で受ける。
// このファイルは Charon / Aeneas で Lean に落として model (lean-handoff の Handoff/Model.lean) との適合を証明する対象なので、
// 文字列・I/O・std の大半を持ち込まない。CLI と文字列の解釈は main.rs に置く (証明の外)。

#[derive(Clone, Copy)]
pub enum Tag {
    Escalation,
    Request,
    Alert,
    Completion,
    Progress,
    Reply,
    // 以下 3 つは model の Tag に無い。語彙と返信要否の正は herdr-message の SKILL.md「本文の書式」
    Info,
    Conflict,
    // 語彙外 (none、打ち間違い)。返信必須に倒す
    Other,
}

// inbox の置き場 (new/ hold/ cur/)
#[derive(Clone, Copy)]
pub enum Place {
    New,
    Held,
    Cur,
}

#[derive(Clone, Copy)]
pub enum Res {
    None,
    Replied,
    NoReply,
    SenderGone,
}

#[derive(Clone, Copy)]
pub struct Msg {
    pub tag: Tag,
    pub sender: u32,
    pub place: Place,
    pub res: Res,
}

#[derive(Clone, Copy)]
pub enum Op {
    Ack,
    AckNoReply,
    AckSenderGone,
    Reply,
    Hold,
}

// sender_state の 0 / 1 / 2
#[derive(Clone, Copy)]
pub enum Sender {
    Alive,
    Gone,
    Unreadable,
}

#[derive(Clone, Copy)]
pub struct Obs {
    pub sender: Sender,
    // reply の送信が成功した。cue_ready (exit 6) の拒否は送信前に外殻が断るので、ここには来ない
    pub sent: bool,
    // read_from が 0 を返した
    pub header_valid: bool,
}

pub enum Refusal {
    // exit 5
    NeedsReply,
    // exit 5
    SenderAlive,
    // exit 1
    SenderUnknown,
    // exit 3
    BadHeader,
    // exit 3
    InCur,
    // 送信側の rc をそのまま返す
    SendFailed,
}

// Moved / Unchanged は exit 0
pub enum Outcome {
    Moved(Msg),
    Unchanged,
    Refused(Refusal),
}

// 返信不要と決まっているタグの補集合
pub fn needs_reply(t: Tag) -> bool {
    match t {
        Tag::Completion | Tag::Progress | Tag::Reply | Tag::Info | Tag::Conflict => false,
        _ => true,
    }
}

fn to_cur(m: Msg, r: Res) -> Outcome {
    Outcome::Moved(Msg { tag: m.tag, sender: m.sender, place: Place::Cur, res: r })
}

pub fn step(m: Msg, op: Op, o: Obs) -> Outcome {
    match op {
        Op::Hold => match m.place {
            Place::New => Outcome::Moved(Msg { tag: m.tag, sender: m.sender, place: Place::Held, res: m.res }),
            // .wait の書き換えだけ
            Place::Held => Outcome::Unchanged,
            Place::Cur => Outcome::Refused(Refusal::InCur),
        },
        // 送ってから cur 判定をするので、cur への reply も送信は起きる
        Op::Reply => {
            if !o.header_valid {
                Outcome::Refused(Refusal::BadHeader)
            } else if !o.sent {
                Outcome::Refused(Refusal::SendFailed)
            } else {
                match m.place {
                    Place::Cur => Outcome::Unchanged,
                    _ => to_cur(m, Res::Replied),
                }
            }
        }
        // ack 系は二重 ack を成功として扱うため、cur の判定がどの検査よりも先
        _ => match m.place {
            Place::Cur => Outcome::Unchanged,
            _ => match op {
                Op::AckSenderGone => {
                    if !o.header_valid {
                        Outcome::Refused(Refusal::BadHeader)
                    } else {
                        match o.sender {
                            Sender::Gone => to_cur(m, Res::SenderGone),
                            Sender::Alive => Outcome::Refused(Refusal::SenderAlive),
                            Sender::Unreadable => Outcome::Refused(Refusal::SenderUnknown),
                        }
                    }
                }
                // escalation / request はヘッダが不正でも閉じない。返信先が無いまま no-reply で閉じると
                // 返信必須の msg が Resolved でない形で cur に入り、model の E4 を破る (w8R の bad_header_breaks_e4)
                Op::AckNoReply => match m.tag {
                    Tag::Escalation | Tag::Request => Outcome::Refused(Refusal::NeedsReply),
                    _ => to_cur(m, Res::NoReply),
                },
                _ => {
                    if needs_reply(m.tag) {
                        Outcome::Refused(Refusal::NeedsReply)
                    } else {
                        to_cur(m, Res::None)
                    }
                }
            },
        },
    }
}

// 以下は step の結果を外殻が実行する副作用の列に直したもの。step の定理はこれに依らない

pub enum Log {
    NoReply,
    Held,
    Replied,
}

pub enum Effect {
    AppendLog(Log),
    MoveTo(Place),
    RemoveWait,
    WriteWait,
}

fn push_move(v: &mut Vec<Effect>, from: Place, to: Place) {
    v.push(Effect::MoveTo(to));
    // hold/ から出るときは待ち先の .wait も消す (to_cur)
    if let Place::Held = from {
        v.push(Effect::RemoveWait);
    }
}

pub fn plan(m: Msg, op: Op, o: Obs) -> Vec<Effect> {
    let mut v = Vec::new();
    match step(m, op, o) {
        Outcome::Moved(_) => match op {
            Op::Ack => push_move(&mut v, m.place, Place::Cur),
            // 理由のログを先に書く。書けなければ移さない
            Op::AckNoReply | Op::AckSenderGone => {
                v.push(Effect::AppendLog(Log::NoReply));
                push_move(&mut v, m.place, Place::Cur);
            }
            Op::Reply => {
                push_move(&mut v, m.place, Place::Cur);
                v.push(Effect::AppendLog(Log::Replied));
            }
            Op::Hold => {
                v.push(Effect::WriteWait);
                v.push(Effect::MoveTo(Place::Held));
                v.push(Effect::AppendLog(Log::Held));
            }
        },
        Outcome::Unchanged => match op {
            Op::Hold => v.push(Effect::WriteWait),
            // cur への reply も送れていれば replied.log に残る
            Op::Reply => v.push(Effect::AppendLog(Log::Replied)),
            _ => {}
        },
        Outcome::Refused(_) => {}
    }
    v
}
