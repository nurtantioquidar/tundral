object MainForm: TMainForm
  Caption = 'Shapes'
  OnCreate = FormCreate
  object Panel1: TPanel
    Align = alTop
    object OkButton: TButton
      Caption = 'OK'
      OnClick = OkButtonClick
    end
    object CancelButton: TButton
      OnClick = CancelButtonClick
    end
  end
  object Memo: TMemo
    Lines.Strings = (
      'first'
      'second')
    OnChange = MemoChange
  end
end
