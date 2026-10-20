type Lifecycle is Data:
  Detached{pid: U32}
  Attached{pid: U32}
  Stopping{pid: U32}
  Stopped{pid: U32}

type BeforeReap is Data:
  Attach{}
  Detach{}
  BeginStop{}

def attach(state: Lifecycle) -> Lifecycle:
  match state:
    case Detached{pid}:
      Attached{pid}
    case Attached{pid}:
      Attached{pid}
    case Stopping{pid}:
      Stopping{pid}
    case Stopped{pid}:
      Stopped{pid}

def detach(state: Lifecycle) -> Lifecycle:
  match state:
    case Detached{pid}:
      Detached{pid}
    case Attached{pid}:
      Detached{pid}
    case Stopping{pid}:
      Stopping{pid}
    case Stopped{pid}:
      Stopped{pid}

def begin_stop(state: Lifecycle) -> Lifecycle:
  match state:
    case Detached{pid}:
      Stopping{pid}
    case Attached{pid}:
      Stopping{pid}
    case Stopping{pid}:
      Stopping{pid}
    case Stopped{pid}:
      Stopped{pid}

def advance(state: Lifecycle, event: BeforeReap) -> Lifecycle:
  match event:
    case Attach{}:
      attach(state)
    case Detach{}:
      detach(state)
    case BeginStop{}:
      begin_stop(state)

def replay(events: List<&2, BeforeReap>, state: Lifecycle) -> Lifecycle:
  match events:
    case Nil{}:
      state
    case Con{event, rest}:
      replay(rest, advance(state, event))

def is_stopped(state: Lifecycle) -> Bool:
  match state:
    case Stopped{pid}:
      True{}
    case _:
      False{}

def finish_stop(state: Lifecycle) -> Lifecycle:
  match state:
    case Detached{pid}:
      Detached{pid}
    case Attached{pid}:
      Attached{pid}
    case Stopping{pid}:
      Stopped{pid}
    case Stopped{pid}:
      Stopped{pid}
