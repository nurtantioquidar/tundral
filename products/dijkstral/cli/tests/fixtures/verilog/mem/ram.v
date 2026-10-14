module ram #(parameter DEPTH = 8) (input clk, input [7:0] d);
  reg [7:0] mem [0:DEPTH-1];
  always @(posedge clk) mem[0] <= d;
endmodule

module dup(input clk);
endmodule
