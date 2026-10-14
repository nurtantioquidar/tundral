/* WHY: arithmetic lives apart from the datapath glue. */
module alu(input [7:0] a, input [7:0] b, output [7:0] s);
  assign s = a + b;

  function automatic [7:0] saturate(input [8:0] v);
    saturate = v[8] ? 8'hff : v[7:0];
  endfunction
endmodule

package alu_pkg;
  function int scale(int v);
    return v * 4;
  endfunction
  task automatic log_op(input int op);
  endtask
endpackage

interface alu_if(input clk);
  logic [7:0] a;
  function automatic logic ready();
    return 1;
  endfunction
endinterface
