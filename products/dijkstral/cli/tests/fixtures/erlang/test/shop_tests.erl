-module(shop_tests).
-include_lib("eunit/include/eunit.hrl").

normalize_test() ->
    ?assertEqual("ada", shop_util:normalize("ADA")).
