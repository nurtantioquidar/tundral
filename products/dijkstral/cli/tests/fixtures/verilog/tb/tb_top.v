// Testbench classes: the class scan strips this comment,
// so every class line below drifts up by its length.
class base_txn;
  function void show();
  endfunction
endclass

class alu_txn extends base_txn implements printable, cloneable;
  rand bit [7:0] a;
  function automatic void randomize_all();
  endfunction
  virtual function void show();
  endfunction
  static function int width(int bits);
    return bits;
  endfunction
  task drive();
  endtask
endclass

module tb_top;
  reg clk;
  top dut(.clk(clk));
  initial clk = 0;
endmodule

typedef class later_txn;
class later_txn;
  function void pending();
  endfunction
endclass
