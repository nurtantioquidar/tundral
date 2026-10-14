-module(shop).
-behaviour(gen_server).

-include("shop.hrl").

-export([start_link/0, register/2]).
-export([init/1, handle_call/3, handle_cast/2]).

-import(shop_util, [normalize/1]).

%% NOTE: the registry lives in the server state.
start_link() ->
    gen_server:start_link({local, ?MODULE}, ?MODULE, [], []).

register(Name, Email) ->
    gen_server:call(?MODULE, {register, #user{name = Name, email = normalize(Email)}}).

init([]) ->
    {ok, []}.

handle_call({register, User}, _From, Users) when is_record(User, user) ->
    {reply, ok, [User | Users]};
handle_call(_Other, _From, Users) ->
    {reply, {error, unknown}, Users}.

handle_cast(_Msg, Users) ->
    {noreply, Users}.
