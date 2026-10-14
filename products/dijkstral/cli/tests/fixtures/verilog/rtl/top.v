// NOTE: top-level wiring; comments never become rationale nodes.
`include "defs.vh"
`define WIDTH 8

module top #(parameter N = 4) (input clk, input rst, output reg [7:0] q);
  wire [7:0] a, b, s;
  reg done;

  alu u_alu(.a(a), .b(b), .s(s));
  fifo #(.DEPTH(16)) u_fifo(.clk(clk), .din(s));
  top_ctrl u_ctrl(.clk(clk), .done(done));
  blackbox_ip u_ip(.clk(clk));
  counter u_cnt(.clk(clk), .q(done));
  dup u_dup(.clk(clk));
  ref_model u_ref(.clk(clk));
  // The grammar reads this one as a UDP instance: no edge.
  alu u_alu2(a, b, s);

  genvar i;
  generate
    for (i = 0; i < N; i = i + 1) begin : gen_lanes
      // Read as an interface instance inside a loop: no edge.
      lane u_lane(.d(a[i]));
    end
    if (N > 2) begin : gen_wide
      wide_path u_wide(.clk(clk));
    end
  endgenerate

  function integer twice(input integer v);
    twice = v * 2;
  endfunction

  task reset_all;
    begin
      done = 0;
    end
  endtask

  always @(posedge clk) begin
    if (rst) reset_all;
    q <= twice(q) + scale(q);
  end
endmodule

module top_ctrl(input clk, output reg done);
  always @(posedge clk) done <= 1'b1;
endmodule

(* keep_hierarchy *) module wide_path(input clk);
endmodule
