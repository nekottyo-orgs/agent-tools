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

// ---- lane と停止 (agents の task-reconcile.sh / frontdesk-health.sh の判定、design.md の S3 / S7) ----
// 事実は外殻が inbox・送信記録・Cue・herdr から task ごとに組んで渡す。時刻はすべて epoch 秒で、外殻が現在時刻も渡す。
// 事実の型には reconciler が書く欄 (lane と機械の欄) を入れない (監査 P1: 自分の書いた印が次の guard を外して往復した)。
// 現在の lane は next_lane の別の引数にする。今の連鎖は同じ観測でも lane ごとに動きが違い (human_decision からは todo へ
// 移すが todo と worker では動かさない)、観測だけでは再現できないため (窓口の決定 O1)

#[derive(Clone, Copy)]
pub enum Lane {
    Todo,
    Worker,
    HumanDecision,
    HumanAction,
    Done,
}

// task の pane 欄から抜いた pane id の観測
#[derive(Clone, Copy)]
pub enum Panes {
    // pane id が 1 つも無い
    Unknown,
    // 全て herdr の一覧に無い
    Absent,
    // 生きていない (一覧に無いか、この実行で閉じた) が、一覧に残るものがある
    Closed,
    // 1 つ以上が一覧にあり、この実行で閉じていない
    Live,
}

#[derive(Clone, Copy)]
pub enum Remaining {
    Empty,
    // 「なし」で始まる
    Nothing,
    Other,
}

#[derive(Clone, Copy)]
pub struct TaskFacts {
    pub panes: Panes,
    // 全 pane が一覧にあり idle / done
    pub idle: bool,
    // working の pane がある
    pub working: bool,
    // この実行で close に失敗した pane がある
    pub close_failed: bool,
    // inbox/cur に task の作成より後の、担当 pane からの completion がある (判定 8)
    pub accepted: bool,
    // 依頼 id の completion を、最後の送信より後に受け入れ済み (判定 12)
    pub req_accepted: bool,
    pub remaining: Remaining,
    // 人の判断待ち (未回答の decision_id か、remaining にある choice の無い Cue)
    pub waiting: bool,
    // decision_id の判断に choice がある
    pub answered: bool,
    // 停止の最初の検出 (stall-detected.log) がある
    pub detected: bool,
    pub detected_at: u64,
    // health-stalled-seen に今も載る
    pub seen: bool,
    // 検出の後に同じ依頼の報告か、催促でない送信がある
    pub cleared: bool,
}

// どの判定で移すか。外殻は理由の文言をこれで選ぶ
#[derive(Clone, Copy)]
pub enum Rule {
    // 判定 8: 閉じた pane の受け入れ済み
    SettleGone,
    // 判定 12: 生きた idle / done pane の受け入れ済み
    SettleDone,
    // 判定 1: 担当 pane 無し
    PaneGone,
    // 判定 2: Cue 回答済み
    Answered,
    // 判定 10: 人の判断待ち
    Waiting,
    // 判定 13: 最初の検出から上限 T
    StallCap,
    // 判定 11: pane が生きている
    Live,
}

#[derive(Clone, Copy)]
pub struct Move {
    pub rule: Rule,
    pub dest: Lane,
}

pub fn over_cap(f: TaskFacts, now: u64, cap_min: u64) -> bool {
    f.detected && f.seen && !f.cleared && now >= f.detected_at + cap_min * 60
}

fn todo_or_worker(l: Lane) -> bool {
    matches!(l, Lane::Todo | Lane::Worker)
}

// 上から順に最初に当たった判定で移す。None なら移さない (外殻は link / review の判定へ進む)。
// done / human_action からは移さない (done は戻せず、human_action から戻すのは人の lane 操作)
pub fn next_lane(from: Lane, f: TaskFacts, now: u64, cap_min: u64) -> Option<Move> {
    let tw = todo_or_worker(from);
    let live = matches!(f.panes, Panes::Live);
    // 各判定の条件は 1 つの bool に先に畳む。&& を if の連鎖に直接書くと Aeneas の出力が枝ごとに膨らみ Lean の elaborate が間に合わない
    let gone = matches!(f.panes, Panes::Absent | Panes::Closed);
    let c_settle_gone = tw && gone && f.accepted;
    let c_settle_done = tw
        && live
        && f.idle
        && f.req_accepted
        && !f.waiting
        && !f.close_failed
        && matches!(f.remaining, Remaining::Nothing);
    let c_pane_gone = matches!(from, Lane::Worker) && matches!(f.panes, Panes::Absent);
    let c_answered = matches!(from, Lane::HumanDecision) && f.answered && !f.waiting;
    let c_waiting = tw && live && f.waiting;
    let c_stall = matches!(from, Lane::Worker) && live && f.idle && over_cap(f, now, cap_min);
    // 回答済みの判断を持ち working でない task は todo に残す (窓口の返事待ち)
    let c_live = matches!(from, Lane::Todo) && live && !(f.answered && !f.working);
    if c_settle_gone {
        let dest = match f.remaining {
            Remaining::Empty | Remaining::Nothing => Lane::Done,
            Remaining::Other => {
                if f.waiting {
                    Lane::HumanDecision
                } else {
                    Lane::HumanAction
                }
            }
        };
        Some(Move { rule: Rule::SettleGone, dest })
    } else if c_settle_done {
        Some(Move { rule: Rule::SettleDone, dest: Lane::Done })
    } else if c_pane_gone {
        Some(Move { rule: Rule::PaneGone, dest: Lane::Todo })
    } else if c_answered {
        Some(Move { rule: Rule::Answered, dest: Lane::Todo })
    } else if c_waiting {
        Some(Move { rule: Rule::Waiting, dest: Lane::HumanDecision })
    } else if c_stall {
        Some(Move { rule: Rule::StallCap, dest: Lane::HumanAction })
    } else if c_live {
        Some(Move { rule: Rule::Live, dest: Lane::Worker })
    } else {
        None
    }
}

// frontdesk-health.sh の停止 (h)。起点は最後の報告と割り当て (task の作成) の新しい方
#[derive(Clone, Copy)]
pub struct StallFacts {
    // pane が 1 つ以上見つかり、全て done / idle
    pub idle: bool,
    // 最後の報告 (無ければ 0)
    pub report: u64,
    // 割り当て (読めなければ 0)
    pub assign: u64,
}

#[derive(Clone, Copy)]
pub enum Origin {
    Report,
    Assign,
}

#[derive(Clone, Copy)]
pub enum Stall {
    // 停止を判定しない (pane が動いているか、起点が無い)
    Active,
    // まだ N 分経たない。値は停止に達する時刻
    Due(u64),
    // 起点から N 分以上報告が無い
    Stalled(Origin),
}

pub fn stalled(f: StallFacts, now: u64, min: u64) -> Stall {
    let t0 = if f.report >= f.assign { f.report } else { f.assign };
    if !f.idle || t0 == 0 {
        Stall::Active
    } else if now < t0 + min * 60 {
        Stall::Due(t0 + min * 60)
    } else if f.report >= f.assign {
        Stall::Stalled(Origin::Report)
    } else {
        Stall::Stalled(Origin::Assign)
    }
}

// 再送の段 (S7)。task-reconcile.sh の resend_step の引数と出力の段名をそのまま境界にする。
// どの 1 通を見るか、台帳の読み直しと 3 周の進め方、送る直前の入力欄の読み直しは外殻に残る
#[derive(Clone, Copy)]
pub enum Agent {
    Claude,
    Other,
}

#[derive(Clone, Copy)]
pub enum Input {
    Empty,
    Draft,
    Unknown,
}

// transcript で読んだ、最後の 1 通が実行されたか
#[derive(Clone, Copy)]
pub enum Delivered {
    Undelivered,
    Executed,
    Uncertain,
}

#[derive(Clone, Copy)]
pub struct ResendFacts {
    // この 1 通に段 1 を使った
    pub tried: bool,
    // 最後の 1 通が今回の検出より後の催促
    pub last_nudge: bool,
    // 今回の検出より後に催促を送った
    pub nudge_sent: bool,
    // 催促を打てなかった記録がある (2 draft / codex / unreadable / overdue)
    pub blocked: bool,
    pub agent: Agent,
    // 送信時の確認で Enter を使った (rc 7)
    pub enter_used: bool,
    pub input: Input,
    // 入力欄の 1 行目が自分の本文
    pub own: bool,
    pub pasted: bool,
    pub delivered: Delivered,
    // 最後の 1 通が上限 T を越えた (D7)
    pub overdue: bool,
}

#[derive(Clone, Copy)]
pub enum Skip {
    Codex,
    EnterUsed,
    Unreadable,
    Empty,
    Pasted,
    Other,
    Restored,
    Uncertain,
}

#[derive(Clone, Copy)]
pub enum Block {
    Codex,
    Overdue,
    Draft,
}

#[derive(Clone, Copy)]
pub enum ResendStep {
    // 1a
    Enter,
    // 1b:<理由>
    Check(Skip),
    Nudge,
    // block:<理由>
    Block(Block),
    Exhausted,
    // none
    Wait,
}

pub fn resend_step(f: ResendFacts) -> ResendStep {
    let claude = matches!(f.agent, Agent::Claude);
    if f.tried {
        if f.last_nudge || f.nudge_sent || f.blocked {
            ResendStep::Exhausted
        } else if !claude {
            ResendStep::Block(Block::Codex)
        } else if f.overdue {
            // D7: 上限 T を越えた最後の 1 通には催促しない
            ResendStep::Block(Block::Overdue)
        } else {
            match f.input {
                Input::Empty => ResendStep::Nudge,
                // agent prompt は下書きに連結して送るので、下書きがあれば催促しない
                Input::Draft => ResendStep::Block(Block::Draft),
                Input::Unknown => ResendStep::Wait,
            }
        }
    } else if !claude {
        ResendStep::Check(Skip::Codex)
    } else if f.enter_used {
        ResendStep::Check(Skip::EnterUsed)
    } else {
        match f.input {
            Input::Unknown => ResendStep::Check(Skip::Unreadable),
            Input::Empty => ResendStep::Check(Skip::Empty),
            Input::Draft => {
                if f.pasted {
                    ResendStep::Check(Skip::Pasted)
                } else if !f.own {
                    ResendStep::Check(Skip::Other)
                } else {
                    match f.delivered {
                        Delivered::Undelivered => ResendStep::Enter,
                        // Claude Code は割り込みで実行済みの文を入力欄に戻す。照合だけで Enter を送ると 2 回実行される
                        Delivered::Executed => ResendStep::Check(Skip::Restored),
                        Delivered::Uncertain => ResendStep::Check(Skip::Uncertain),
                    }
                }
            }
        }
    }
}

