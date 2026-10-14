-module(shop_util).
-export([normalize/1]).

%% WHY: emails compare case-insensitively.
normalize(undefined) -> undefined;
normalize(Email) -> string:lowercase(Email).
