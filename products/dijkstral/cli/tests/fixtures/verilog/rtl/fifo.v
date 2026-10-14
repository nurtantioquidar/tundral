module fifo #(parameter DEPTH = 8) (input clk, input [7:0] din, output [7:0] dout);
  ram #(.DEPTH(DEPTH)) u_ram0(.clk(clk), .d(din));
  ram #(.DEPTH(DEPTH)) u_ram1(.clk(clk), .d(din));
  blackbox_ip u_ip(.clk(clk));

  module fifo_flags(input clk);
    ram u_flag_ram(.clk(clk));
  endmodule
endmodule

module dup(input clk);
endmodule
